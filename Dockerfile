FROM rust:1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p api-server

FROM debian:bookworm-slim
COPY --from=build /src/target/release/api-server /usr/local/bin/api-server
ENV LN_API_BIND=container
EXPOSE 3001 9735
CMD ["api-server"]
