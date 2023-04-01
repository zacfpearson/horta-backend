sudo docker build -f docker/Dockerfile.dev -t horta-backend:dev docker
sudo docker run -it --rm --mount type=bind,source=$(pwd)/code,target=/build horta-backend:dev /bin/bash -c "cargo build"
sudo docker build --no-cache -f docker/Dockerfile.serve -t horta-backend:serve code
sudo docker run --name horta-backend --rm --network=host horta-backend:serve