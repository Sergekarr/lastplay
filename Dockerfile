
FROM rust:1.80


COPY ./ ./

RUN cargo build --release

CMD ["./target/release/lastplay"]

