/// aggregate macro must reject enums.

#[mnesis::aggregate(state = (), error = (), id = ())]
enum NotAStruct {
    A,
    B,
}

fn main() {}
