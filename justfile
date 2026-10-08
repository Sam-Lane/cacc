# list all just recipes
@default:
    just -l

# format all code
fmt:
    cargo fmt --all

# run clippy as CI does
clippy:
    cargo clippy --all-targets --all-features -- -D warnings

# run all tests
test:
    cargo test --all-features
