/// aggregate macro must require `id`.

#[mnesis::aggregate(state = (), error = std::io::Error)]
struct MissingId;

fn main() {}
