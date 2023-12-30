use uuid::Uuid;

#[derive(Debug)]
struct Citation {
    id: Uuid,
    title: String,
    label: String,
    text: String,
}

#[derive(Debug)]
struct Hyperedge {
    id: Uuid,
    label: String,
    source: Vec<Node>,
    target: Vec<Node>,
    citations: Vec<Citation>,
}

#[derive(Debug)]
struct Node {
    id: Uuid,
    label: String,
}

fn main() {
    println!("Hello, world!");
}
