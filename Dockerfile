FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY server /usr/local/bin/server
COPY client /client

RUN chmod +x /usr/local/bin/server

EXPOSE 7903

CMD ["/usr/local/bin/server"]
