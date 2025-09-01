#!/bin/bash
cargo lambda build --release --bin orderbook-lambda
cargo lambda build --release --bin recovery
sam deploy
