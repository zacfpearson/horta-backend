#![deny(warnings)]
use futures_util::{FutureExt, StreamExt};
use pretty_env_logger;
use rand::prelude::*;
use serde::{Deserialize, Serialize}; 
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Condvar};
use std::thread;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use warp::{http::StatusCode, ws::{Message, WebSocket}, Filter, Rejection, Reply};
use uuid::Uuid;

type Result<T> = std::result::Result<T, Rejection>;
type Games = Arc<Mutex<HashMap<u64, Game>>>;

#[derive(Serialize, Deserialize, Eq, PartialEq, Clone)]
pub enum Player {
    Computer,
    Person
}

#[derive(Serialize, Deserialize, Eq, PartialEq, Clone)]
pub struct Card {
    player: Player,
    number: u8,
}

impl Card {
    fn new(number: u8, player: Player) -> Card {
        Card{ number: number, player: player }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Game {
    level: u8,
    cards: Vec<Card>,
    cards_played: Vec<Card>,
}

impl Game {
    fn new(level: u8) -> Game {
        let mut rng = rand::thread_rng();

        //get unique set of random numbers
        let mut numbers: Vec<u8> = Vec::new();
        for _ in 0..level*2 {
            let mut number = rng.gen_range(0..100);
            while numbers.contains(&number) {
                number = rng.gen_range(0..100);
            }
            numbers.push(number);
        }

        //group inot person and computer cards.
        let mut cards: Vec<Card> = Vec::new();
        let split_numbers: Vec<&[u8]> = numbers.chunks(7).collect();
        for number in split_numbers[0] {
            cards.push(Card::new(*number, Player::Computer));
        }   

        for number in split_numbers[1] {
            cards.push(Card::new(*number, Player::Person));
        }

        cards.sort_by(|a, b| b.number.cmp(&a.number));
        cards.reverse();

        Game{ level: level, cards: cards, cards_played: Vec::new() }
    }

    pub fn next_card(&self, player: Player) -> Option<Card> {
        self.cards
            .iter()
            .filter(|&x| !self.cards_played.contains(&x))
            .find(|&x| x.player == player)
            .map(|x| x.clone()) //TODO: try to remove clone here. Needed because the reference to the card keeps the borrow to the 
    }

    pub fn current_difference(&self, card: &Card) -> u8 {
        if let Some(last_card) = self.cards_played.last() {
            card.number - last_card.number
        } else {
            card.number
        }
    }
}

async fn games_handler(games: Games) -> Result<impl Reply> {
    //get random index
    let index = rand::thread_rng().gen_range(0..50);
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

    let game_computer: Arc<(Mutex<Game>,Condvar)> = Arc::new((Mutex::new(game), Condvar::new()));
    let game_person = game_computer.clone();

    tokio::task::spawn(
        rx.forward(tx_ws).map(|result| {
        if let Err(e) = result {
            eprintln!("error sending websocket msg: {}", e);
        }
    }));

    //the computer playing.
    let send_to_reciever = thread::spawn(move || {
        let (lock, cvar) = &*game_computer;
        println!("have lock var");
        let mut game_instance = lock.lock().unwrap();
        while let Some(card) = game_instance.next_card(Player::Computer) {
            let current_difference = game_instance.current_difference(&card) as u64;

            println!("have lock.Going to sleep for: {}", (current_difference));
            
            let sleep_time = if game_instance.next_card(Player::Person) == None {
                400
            } else {
                current_difference * 2000
            };

            println!("awake and ready.");

            let result = cvar.wait_timeout(game_instance, Duration::from_millis(sleep_time as u64)).unwrap();

            println!("have result");

            game_instance = result.0;

            if result.1.timed_out() {
                println!("timeout. Nex card: {}", card.number);
                let encoded: Vec<u8> = bincode::serialize(&card).unwrap();
                println!("Encoding");
                game_instance.cards_played.push(card);
                let _ = tx.send(Ok(Message::binary(encoded)));
                println!("Sent!");
            }
        }
        println!("All Done Sending");
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
                    let &(ref lock, ref cvar) = &*game_person;

                    println!("getting card lock");
                    let mut started = lock.lock().unwrap();

                    started.cards_played.push(card);
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
