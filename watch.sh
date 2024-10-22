#!/bin/bash
RUST_LOG=debug RUST_BACKTRACE=1 cargo watch -x check -x test -x run
