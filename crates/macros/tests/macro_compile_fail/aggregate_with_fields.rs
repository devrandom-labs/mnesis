/// aggregate macro must reject structs with fields.

#[mnesis::aggregate(state = (), error = (), id = ())]
struct HasFields {
    name: String,
}

fn main() {}
