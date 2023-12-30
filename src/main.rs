use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use openai_api_rs::v1::common::GPT4;
use std::env;
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

fn get_openai_response(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new(env::var("OPENAI_API_KEY")?);

    let req = ChatCompletionRequest::new(
        GPT4.to_string(),
        vec![chat_completion::ChatCompletionMessage {
            role: chat_completion::MessageRole::user,
            content: prompt,
            name: None,
            function_call: None,
        }],
    );

    let result = client.chat_completion(req)?;

    // Handling the Option<String> with ok_or
    result.choices.get(0)
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| "No response from OpenAI".into()) // Converting to Result
}

fn main() {
    println!("Hello, world!");
    get_openai_response(String::from("What is an ontology?"))
        .map(|response| println!("{}", response))
        .map_err(|_| println!("Error with OpenAI API"))
        .unwrap_or(());
}
