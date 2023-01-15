sudo docker build -f docker/Dockerfile.dev -t sentiment-backend:dev docker
sudo docker run -it --mount type=bind,source=$(pwd)/code,target=/build sentiment-backend:dev /bin/bash -c "cargo build --release"
sudo docker build -f docker/Dockerfile.serve -t sentiment-backend:serve code
sudo docker run --name sentiment-backend --rm --network=host sentiment-backend:serve