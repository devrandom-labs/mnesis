/// aggregate macro must require `state`.

#[mnesis::aggregate(error = std::io::Error, id = u64)]
struct MissingState;

fn main() {}
