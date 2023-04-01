
#![deny(warnings)]

use futures_util::{FutureExt, StreamExt};
use warp::Filter;
use pretty_env_logger;
use serde::{Deserialize, Serialize}; 
use warp::{ http::StatusCode, ws::{Message, WebSocket}, Rejection, Reply};

use std::sync::{Arc, Mutex, Condvar};
use std::thread;
use tokio::sync::mpsc;
use std::time::Duration;

use std::sync::atomic::{AtomicBool, Ordering};

use rand::prelude::*;

use tokio_stream::wrappers::UnboundedReceiverStream;

use std::collections::HashMap;

type Result<T> = std::result::Result<T, Rejection>;

use uuid::Uuid;

// use std::hash::Hash;

//TODO: looke into something other than hashmap? And use.next() to get element rathert than random ID?
type Games = Arc<Mutex<HashMap<u64, Game>>>;

//TODO: maybe use this as a test? 
// fn has_unique_elements<T>(iter: T) -> bool
// where
//     T: IntoIterator,
//     T::Item: Eq + Hash,
// {
//     let mut uniq = HashSet::new();
//     iter.into_iter().all(move |x| uniq.insert(x))
// }

#[derive(Serialize, Deserialize, Debug, Copy, Clone, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Card {
    number: u8,
}

impl Card {
    fn new(number: u8) -> Card {
        Card{ number: number }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Hand {
    cards: Vec<Card>,
    last_index_played: usize, //TODO: I don't like this. Try a different way later=
}

impl Hand {
    fn blank() -> Hand {
        Hand{ cards: Vec::new(), last_index_played: 0}
    }

    fn add_card(&mut self, card: Card) {
        self.cards.push(card);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Guesses {
    cards: Vec<Card>,
}

impl Guesses {
    fn blank() -> Guesses {
        Guesses{ cards: Vec::new() }
    }

    //TODO: Is it needed?
    // fn add_card(&mut self, card: Card) {
    //     self.cards.push(card);
    // }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Player {
    hand: Hand,
    guesses: Guesses,
}

impl Player {
    fn blank() -> Player {
        Player{ hand: Hand::blank(), guesses: Guesses::blank() }
    }

    fn recieve_card(&mut self, card: Card) {
        self.hand.add_card(card);
    }

    fn sort_hand(&mut self) {
        self.hand.cards.sort_by(|a, b| b.number.cmp(&a.number)); //TODO: cleanup
        self.hand.cards.reverse();
    }

    //TODO: is it needed?
    // fn play_card(&mut self, card: Card) {
    //     self.guesses.add_card(card);
    // }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Game {
    level: u8,
    player: Player,
    computer: Player
}

impl Game {
    fn new(level: u8) -> Game {
        let mut rng = rand::thread_rng();
        let mut player = Player::blank();
        let mut computer = Player::blank();

        let mut cards: Vec<Card> = Vec::new();

        for _ in 0..level*2 {
            let mut card = Card::new(rng.gen_range(0..100));
            while cards.contains(&card) {
                card = Card::new(rng.gen_range(0..100));
            }
            cards.push(card);
        }

        // if has_unique_elements(&cards) {
        //     println!("unique!");
        // } else {
        //     println!("not unique!");
        // }

        let split_cards: Vec<&[Card]> = cards.chunks(7).collect();
        for card in split_cards[0] {
            player.recieve_card(*card);
        }   

        for card in split_cards[1] {
            computer.recieve_card(*card);
        }   

        //TODO: Try a cleaner slolution.

        player.sort_hand();
        computer.sort_hand();

        Game{ level: level, player: player, computer: computer }
    }
}

async fn games_handler(games: Games) -> Result<impl Reply> {
    //get random ID
    let index = rand::thread_rng().gen_range(0..50);//get random //TRY SOMETHING ELSE HERE
    if let Some(game_and_id) = games.lock().unwrap().get(&index){
        let uuid = Uuid::new_v4();
        let encoded: Vec<u8> = bincode::serialize(&(uuid.as_u128(),index,game_and_id)).unwrap();
        Ok(encoded)
    } else {
        Err(warp::reject::not_found())
    }
}

pub async fn ws_handler(ws: warp::ws::Ws, _: u128, index: u64, games: Games) -> Result<impl Reply> {
    if let Some(game) = games.lock().unwrap().get(&index) {
        let game_clone = game.clone();
        Ok(ws.on_upgrade(move |socket| game_connection(socket, game_clone)))
    } else {
        Err(warp::reject::not_found())
    }
}

pub async fn game_connection(ws: WebSocket, game: Game) {
    // Just echo all messages back...
    let (tx_ws, mut rx_ws) = ws.split();

    let (tx, rx) = mpsc::unbounded_channel();
    let rx = UnboundedReceiverStream::new(rx);

    let connected: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
    let connection_clone = connected.clone();

    println!("game sent: {:?}", game);

    // let game_pair = Arc::new((Mutex::new(game), Condvar::new())); //TODO: maybe guesses?
    let guesses_computer: Arc<(Mutex<Option<Card>>,Condvar)> = Arc::new((Mutex::new(None), Condvar::new()));
    let guesses_player = guesses_computer.clone();

    let computer_hand = game.computer.hand.cards.clone();

    tokio::task::spawn(
        rx.forward(tx_ws).map(|result| {
        if let Err(e) = result {
            eprintln!("error sending websocket msg: {}", e);
        }
    }));

    //the cumputer playing.
    let send_to_reciever = thread::spawn(move || {
        let mut hand_iter =  computer_hand.iter().peekable();
        let mut last_card_played = Card{number: 0};
        let mut player_played_cards = Vec::new();

        while connection_clone.load(Ordering::Relaxed) {
            if let Some(computer_card) = hand_iter.peek() {
                println!("have another card");
                let (lock, cvar) = &*guesses_computer;
                println!("have lock var");
                let mut card_played = lock.lock().unwrap();

                //TODO: check ammount of cards user played, if the user has no more cards left, play cards left quickly after the other. 
                println!("have lock.Going to sleep for: {}", (computer_card.number - last_card_played.number));
                
                //If the user has played all of their cards and we haven't lost(we are connected over WSS), just send all of our cards. 
                //TODO: probably a better way of checking if the user has played all of there cards
                let sleep_time = if player_played_cards.len() > 6 { 400 } else {(computer_card.number - last_card_played.number) as u64 * 2000}; //TODO: Change sleep time back to 500 or so, jsut higher for testing

                println!("awake and ready.");

                let result = cvar.wait_timeout(card_played, Duration::from_millis(sleep_time)).unwrap();

                println!("have result");

                card_played = result.0;

                if !result.1.timed_out() {
                    println!("No Time out");
                    if let Some(card) = *card_played {
                        println!("No time out card: {}", card.number);
                        player_played_cards.push(card);
                        last_card_played = card;
                    }

                } else {
                    if let Some(card) = hand_iter.next() {
                        println!("timeout. Nex card: {}", card.number);
                        last_card_played = *card;
                        let encoded: Vec<u8> = bincode::serialize(&last_card_played).unwrap();

                        println!("Encoding");

                        let _ = tx.send(Ok(Message::binary(encoded)));
                        println!("Sent!");
                    }
                }

            } else {
                //done playing leave web socket.

                //TODO: maybe a better thing to do here than just break.

                //TODO: Sleeping for now but would really like to just close the connection on our end or just stop processing and wait til the game is over.
                //WEIRD BUG: afte rthe user disconnects, we still are processing this thread.
                println!("all done with our cards! Sleep.");

                thread::sleep(Duration::from_millis(100));
                //break;
            }
        }
    });

    while let Some(result) = rx_ws.next().await {
        let msg = match result {
            Ok(msg) => msg,
            Err(e) => {
                eprintln!("error receiving ws message for {}", e);
                break; //handle differently.
            }
        };

        //TODO: idk if I want all of these prints. 
        if msg.is_binary() {
            println!("deserializing card.");
            match bincode::deserialize(msg.as_bytes()) {
                Ok(card) => {
                    let &(ref lock, ref cvar) = &*guesses_player;

                    println!("getting card lock");
                    let mut started = lock.lock().unwrap();

                    *started = Some(card);
                    println!("set card.");
                    // We notify the condvar that the value has changed.
                    cvar.notify_all();    
                    println!("sent.")
                },
                Err(e) => {
                    println!("error deserializing card: {}", e);
                }
            }
        } else if msg.is_close() {
            println!("closing connection!");
        } else {
            println!("{:?}", msg);
        }
    }

    connected.store(false, Ordering::Relaxed);
    
    send_to_reciever.join().expect("The sender thread has panicked");
}

pub async fn health_handler() -> Result<impl Reply> {
    Ok(StatusCode::OK)
}

fn create_n_games(num_games: u64) -> Games {
    let mut games: HashMap<u64, Game> = HashMap::new();
    for idx in 0..num_games {
        let new_game = Game::new(7); //create new game at level 7
        games.insert(idx, new_game);
    }

    Arc::new(Mutex::new(games))
}

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    //create 50 random games
    let games: Games = create_n_games(50);

    let games = warp::any().map(move || games.clone());

    let health_route = warp::path!("health").and_then(health_handler);

    //TODO: add content length and stuuf and type checks.
    let cards = warp::path("get-cards");
    let card_routes = cards
        .and(warp::get())
        .and(games.clone())
        .and_then(games_handler)
        .or(
            cards.and(warp::options().map(warp::reply))
        );

    let ws_route = warp::path("ws")
        .and(warp::ws())
        .and(warp::path::param())
        .and(warp::path::param())
        .and(games.clone())
        .and_then(ws_handler);

    let routes = health_route
        .or(card_routes)
        .or(ws_route)
        .with(warp::cors()
            .allow_any_origin()
            .allow_headers(vec!["Authorization", "X-Requested-With", "Cache-Control", "Accept-Language","Connection","Sec-Fetch-Dest","Sec-Fetch-Site","User-Agent","Host","Sec-Fetch-Mode", "Referer", "Origin", "Access-Control-Request-Method", "Access-Control-Request-Headers","Accept","Content-Type","Access-Control-Allow-Headers","Access-Control-Allow-Methods","Access-Control-Allow-Origin"])
            .allow_methods(vec!["POST", "GET","OPTIONS"]))
            .with(warp::log("cors test"));

    warp::serve(routes).run(([127, 0, 0, 1], 3030)).await;
}
