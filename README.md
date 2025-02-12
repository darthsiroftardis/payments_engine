# Simple Payment engine

A simple stateless processor for transactions, takes in a simple CSV file and outputs the specified client end state based on spec

## Usage

`cargo run -- transactions.csv > accounts.csv`

## Test coverage

The program comes with four basic unit tests that cover the basic scenarios.

```rust
running 4 tests
test tests::should_correctly_hold_based_on_dispute ... ok
test tests::should_correctly_hold_based_on_dispute_and_release_based_on_resolve ... ok
test tests::should_correctly_chargeback ... ok
test tests::should_parse_transactions ... ok
```

## TODOs

Currently the implementation is very simple and doesn't really do much past the bare minimum.
It is currently stateless which means the internal state mutations of clients is somewhat opaque, thus you see tests with repeated code
Making the system actually work as a state machine would give the most bang for buck in terms of test coverage, but given time constraints it wasn't possible to do
Adding persistence as well would be really good, thus if we got half through reading a file, we don't need to start all over again on recovery.