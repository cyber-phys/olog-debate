use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use openai_api_rs::v1::common::GPT4;
use std::io;
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


fn get_openai_response_json(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new(env::var("OPENAI_API_KEY")?);

    let response_format_value = serde_json::json!({ "type": "json_object" });

    let req = ChatCompletionRequest::new(
        "gpt-4-1106-preview".to_string(),
        vec![chat_completion::ChatCompletionMessage {
            role: chat_completion::MessageRole::user,
            content: prompt,
            name: None,
            function_call: None,
        }],
    )
    .response_format(response_format_value); // Set the response_format here

    let result = client.chat_completion(req)?;

    // Handling the Option<String> with ok_or
    result.choices.get(0)
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| "No response from OpenAI".into()) // Converting to Result
}

fn main() {
    let prompt = include_str!("./res/olog.md").to_string();
    let text = include_str!("./res/olog-pdf.md").to_string();
    
    // Make the OpenAI API call using the included prompt
    get_openai_response_json(format!("{}\n{}", prompt, text))
        .map(|response| println!("{}", response))
        .map_err(|e| {
            println!("Error with OpenAI API {}", e);
            io::Error::new(io::ErrorKind::Other, "OpenAI API error")
        })
        .unwrap_or(());
}
