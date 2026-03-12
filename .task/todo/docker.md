# Docker / docker-compose

Package the server for easy deployment.

- `Dockerfile`: multi-stage build (rust:alpine builder → distroless/alpine runner)
- `docker-compose.yml`: single service, port 8421, env via `.env`
- Document in README: `docker compose up`
