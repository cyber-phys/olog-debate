use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use openai_api_rs::v1::common::GPT4;
use std::env;
use uuid::Uuid;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use pyo3::prelude::*;
use pyo3::types::{PyModule, PyTuple, PyString};
use pyo3::conversion::IntoPy;
use pyo3::conversion::ToPyObject;

#[derive(Debug, Serialize, Deserialize)]
struct JsonOlogSchema {
    title: String,
    nodes: Vec<JsonNodeSchema>,
    hyperedges: Vec<JsonHyperedgeSchema>,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonNodeSchema {
    id: String,
    label: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JsonHyperedgeSchema {
    id: String,
    label: String,
    sources: Vec<String>,
    targets: Vec<String>,
}

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
struct Node {
    id: Uuid,
    label: String,
}

#[derive(Debug)]
struct Olog {
    id: Uuid,
    title: String,
    nodes: Vec<Node>,
    hyperedges: Vec<Hyperedge>,
}

fn extract_text_from_pdf(pdf_path: &str, checkpoint_path: &str) -> PyResult<String> {
    Python::with_gil(|py| {
        let nougat_ocr = PyModule::import(py, "nougat_ocr")?;

        // Convert Rust strings to Python objects directly
        let pdf_path_py = pdf_path.to_object(py);
        let checkpoint_path_py = checkpoint_path.to_object(py);

        // Create a Python tuple with explicit Python objects
        let args = PyTuple::new(py, &[pdf_path_py, checkpoint_path_py]);
        let text = nougat_ocr.call_method1("extract_text_from_pdf_file", args)?;
        text.extract()
    })
}



fn validate_olog_schema(json_data: &str) -> Result<(), serde_json::Error> {
    let _olog: JsonOlogSchema = serde_json::from_str(json_data)?;
    Ok(())
}

fn get_openai_response(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new(env::var("OPENAI_API_KEY")?);

    let req = ChatCompletionRequest::new(
        "gpt-4-1106-preview".to_string(),
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


fn replace_ids_with_uuids(mut olog: JsonOlogSchema) -> JsonOlogSchema {
    let mut id_map: HashMap<String, Uuid> = HashMap::new();

    // Replace node ids
    for node in olog.nodes.iter_mut() {
        let uuid = *id_map.entry(node.id.clone()).or_insert_with(Uuid::new_v4);
        node.id = uuid.to_string();
    }

    // Replace hyperedge ids and update sources and targets
    for hyperedge in olog.hyperedges.iter_mut() {
        let uuid = *id_map.entry(hyperedge.id.clone()).or_insert_with(Uuid::new_v4);
        hyperedge.id = uuid.to_string();

        for source_id in hyperedge.sources.iter_mut() {
            let source_uuid = *id_map.entry(source_id.clone()).or_insert_with(Uuid::new_v4);
            *source_id = source_uuid.to_string();
        }

        for target_id in hyperedge.targets.iter_mut() {
            let target_uuid = *id_map.entry(target_id.clone()).or_insert_with(Uuid::new_v4);
            *target_id = target_uuid.to_string();
        }
    }

    olog
}

fn convert_json_olog_to_olog(json_olog: JsonOlogSchema, citation: Citation) -> Olog {
    let mut id_map: HashMap<String, Uuid> = HashMap::new();
    let mut node_map: HashMap<Uuid, Node> = HashMap::new();

    // Process nodes and build a map from string IDs to Node instances
    for json_node in &json_olog.nodes {
        let uuid = *id_map.entry(json_node.id.clone()).or_insert_with(Uuid::new_v4);
        let node = Node { id: uuid, label: json_node.label.clone() };
        node_map.insert(uuid, node);
    }

    // Convert nodes to Vec<Node>
    let nodes: Vec<Node> = node_map.values().cloned().collect();

    // Process hyperedges and convert sources and targets to Node instances
    let hyperedges = json_olog.hyperedges.into_iter().map(|json_hyperedge| {
        let hyperedge_id = *id_map.entry(json_hyperedge.id.clone()).or_insert_with(Uuid::new_v4);
        let sources = json_hyperedge.sources.iter()
            .filter_map(|source_id| {
                id_map.get(source_id)
                    .and_then(|&uuid| node_map.get(&uuid).cloned())
            })
            .collect();
        let targets = json_hyperedge.targets.iter()
            .filter_map(|target_id| {
                id_map.get(target_id)
                    .and_then(|&uuid| node_map.get(&uuid).cloned())
            })
            .collect();

        Hyperedge {
            id: hyperedge_id,
            label: json_hyperedge.label,
            source: sources,
            target: targets,
            citations: vec![citation.clone()],
        }
    }).collect();

    Olog {
        id: Uuid::new_v4(),
        title: json_olog.title,
        nodes,
        hyperedges,
    }
}

fn generate_olog(text: String) -> Result<Olog, Box<dyn std::error::Error>> {
    let prompt = include_str!("./res/olog.md").to_string();
    let openai_response = get_openai_response_json(format!("{}\n{}", prompt, text))?;
    let openai_title = get_openai_response(format!("{}\n{}", "What is the the title of this document? Respond with only the title and no additional text", text))?;
    let openai_label = get_openai_response(format!("{}\n{}", "Create a label for this document. The label should be under 50 words long. Respond with only the label and no additional text", text))?;
    let olog_schema: JsonOlogSchema = serde_json::from_str(&openai_response)?;
    let olog_schema_uuid: JsonOlogSchema = replace_ids_with_uuids(olog_schema);
    let citation: Citation = Citation {
        id: Uuid::new_v4(),
        title: openai_title,
        label: openai_label,
        text: text,
    };
    let olog: Olog = convert_json_olog_to_olog(olog_schema_uuid, citation);

    Ok(olog)
}

fn main() {
    let pdf_path = "/Users/luc/AI/olog/src/res/olog.pdf";
    let checkpoint_path = "/Users/luc/AI/olog/res/nougat/";
    let text = include_str!("./res/olog-pdf.md").to_string();
    //match generate_olog(text) {
    //    Ok(olog_schema) => println!("{:#?}", olog_schema),
    //    Err(e) => println!("An error occurred: {}", e),
    //}
    match extract_text_from_pdf(pdf_path, checkpoint_path) {
        Ok(text) => println!("Extracted Text: {}", text),
        Err(e) => println!("Error: {}", e),
    }
}

