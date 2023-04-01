sudo docker build -f docker/Dockerfile.dev -t sentiment-backend:dev docker
sudo docker run -it --rm --mount type=bind,source=$(pwd)/code,target=/build sentiment-backend:dev /bin/bash -c "cargo build"
sudo docker build --no-cache -f docker/Dockerfile.serve -t sentiment-backend:serve code
sudo docker run --name sentiment-backend --rm --network=host sentiment-backend:serve