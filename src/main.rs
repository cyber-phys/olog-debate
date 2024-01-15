use clap::{App, Arg, SubCommand};
use openai_api_rs::v1::api::Client;
use openai_api_rs::v1::chat_completion::{self, ChatCompletionRequest};
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

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

#[derive(Debug, Clone)]
struct Hyperedge {
    id: Uuid,
    label: String,
    source: Vec<Node>,
    target: Vec<Node>,
    citations: Vec<Citation>,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
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

#[derive(Serialize)]
struct OcrPdfPostRequest {
    version: String,
    input: OcrInput,
}

#[derive(Serialize)]
struct OcrInput {
    document: String,
    postprocess: bool,
    early_stopping: bool,
}

#[derive(Deserialize)]
struct OcrPdfPostResponse {
    urls: OcrUrls,
}

#[derive(Deserialize)]
struct OcrUrls {
    get: String,
}

#[derive(Deserialize)]
struct OcrPdfGetResponse {
    status: String,
    output: Option<String>, // Assuming the output is a string; adjust as needed
}

fn create_olog_tables() -> Result<(), rusqlite::Error> {
    let conn = Connection::open("olog.db")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Ologs (
            olog_id TEXT PRIMARY KEY,
            title TEXT NOT NULL
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Nodes (
            node_id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            olog_id TEXT NOT NULL,
            FOREIGN KEY(olog_id) REFERENCES Ologs(olog_id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Hyperedges (
            hyperedge_id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            olog_id TEXT NOT NULL,
            FOREIGN KEY(olog_id) REFERENCES Ologs(olog_id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Citations (
            citation_id TEXT PRIMARY KEY,
            title TEXT,
            label TEXT,
            text TEXT
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Hyperedge_Links (
            hyperedge_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            type TEXT NOT NULL,
            FOREIGN KEY(hyperedge_id) REFERENCES Hyperedges(hyperedge_id),
            FOREIGN KEY(node_id) REFERENCES Nodes(node_id)
        )",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS Citation_Links (
            hyperedge_id TEXT NOT NULL,
            citation_id TEXT NOT NULL,
            FOREIGN KEY(hyperedge_id) REFERENCES Hyperedges(hyperedge_id),
            FOREIGN KEY(citation_id) REFERENCES Citations(citation_id)
        )",
        [],
    )?;

    Ok(())
}

fn read_olog_from_db(olog_id: Uuid) -> Result<Olog> {
    let conn = Connection::open("olog.db")?;

    let mut stmt = conn.prepare("SELECT title FROM Ologs WHERE olog_id = ?1")?;
    let olog_title: String = stmt.query_row(params![olog_id.to_string()], |row| row.get(0))?;

    let mut stmt = conn.prepare("SELECT node_id, label FROM Nodes WHERE olog_id = ?1")?;
    let nodes_iter = stmt.query_map(params![olog_id.to_string()], |row| {
        let id_str: String = row.get(0)?;
        let id = Uuid::parse_str(&id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(Node {
            id,
            label: row.get(1)?,
        })
    })?;

    let nodes: Vec<Node> = nodes_iter
        .into_iter()
        .filter_map(|result| result.ok()) // Handle each row's result
        .collect();

    let mut stmt = conn.prepare("SELECT hyperedge_id, label FROM Hyperedges WHERE olog_id = ?1")?;
    let hyperedges_iter = stmt.query_map(params![olog_id.to_string()], |row| {
        let hyperedge_id_str: String = row.get(0)?;
        let hyperedge_id =
            Uuid::parse_str(&hyperedge_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;

        let mut stmt = conn.prepare(
            "
            SELECT c.citation_id, c.title, c.label, c.text
            FROM Citations AS c
            JOIN Citation_Links AS cl ON c.citation_id = cl.citation_id
            WHERE cl.hyperedge_id = ?1
        ",
        )?;
        let citations_iter = stmt.query_map(params![hyperedge_id.to_string()], |row| {
            let citation_id_str: String = row.get(0)?;
            let citation_id =
                Uuid::parse_str(&citation_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;

            Ok(Citation {
                id: citation_id,
                title: row.get(1)?,
                label: row.get(2)?,
                text: row.get(3)?,
            })
        })?;

        let citations: Vec<Citation> = citations_iter.into_iter().collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT node_id FROM Hyperedge_Links WHERE hyperedge_id = ?1 AND type = 'source'",
        )?;
        let sources_iter = stmt.query_map(params![hyperedge_id.to_string()], |row| {
            let node_id_str: String = row.get(0)?;
            let node_id =
                Uuid::parse_str(&node_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
            nodes
                .iter()
                .find(|&n| n.id == node_id)
                .cloned()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)
        })?;

        let sources: Vec<Node> = sources_iter.into_iter().collect::<Result<Vec<_>, _>>()?;

        let mut stmt = conn.prepare(
            "SELECT node_id FROM Hyperedge_Links WHERE hyperedge_id = ?1 AND type = 'target'",
        )?;
        let targets_iter = stmt.query_map(params![hyperedge_id.to_string()], |row| {
            let node_id_str: String = row.get(0)?;
            let node_id =
                Uuid::parse_str(&node_id_str).map_err(|_| rusqlite::Error::InvalidQuery)?;
            nodes
                .iter()
                .find(|&n| n.id == node_id)
                .cloned()
                .ok_or(rusqlite::Error::QueryReturnedNoRows)
        })?;

        let targets: Vec<Node> = targets_iter.into_iter().collect::<Result<Vec<_>, _>>()?;

        Ok(Hyperedge {
            id: hyperedge_id,
            label: row.get(1)?,
            source: sources,
            target: targets,
            citations,
        })
    })?;

    let hyperedges: Vec<Hyperedge> = hyperedges_iter.into_iter().collect::<Result<Vec<_>, _>>()?;

    Ok(Olog {
        id: olog_id,
        title: olog_title,
        nodes,
        hyperedges,
    })
}

fn write_olog_to_db(olog: &Olog) -> Result<()> {
    let conn = Connection::open("olog.db")?;

    conn.execute("BEGIN TRANSACTION", [])?;

    conn.execute(
        "INSERT OR REPLACE INTO Ologs (olog_id, title) VALUES (?1, ?2)",
        params![olog.id.to_string(), olog.title],
    )?;

    for node in &olog.nodes {
        conn.execute(
            "INSERT OR REPLACE INTO Nodes (node_id, label, olog_id) VALUES (?1, ?2, ?3)",
            params![node.id.to_string(), node.label, olog.id.to_string()],
        )?;
    }

    for hyperedge in &olog.hyperedges {
        conn.execute(
            "INSERT OR REPLACE INTO Hyperedges (hyperedge_id, label, olog_id) VALUES (?1, ?2, ?3)",
            params![
                hyperedge.id.to_string(),
                hyperedge.label,
                olog.id.to_string()
            ],
        )?;

        for citation in &hyperedge.citations {
            conn.execute(
                "INSERT OR REPLACE INTO Citations (citation_id, title, label, text) VALUES (?1, ?2, ?3, ?4)",
                params![citation.id.to_string(), citation.title, citation.label, citation.text],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO Citation_Links (hyperedge_id, citation_id) VALUES (?1, ?2)",
                params![hyperedge.id.to_string(), citation.id.to_string()],
            )?;
        }

        for source in &hyperedge.source {
            conn.execute(
                "INSERT OR REPLACE INTO Hyperedge_Links (hyperedge_id, node_id, type) VALUES (?1, ?2, 'source')",
                params![hyperedge.id.to_string(), source.id.to_string()],
            )?;
        }

        for target in &hyperedge.target {
            conn.execute(
                "INSERT OR REPLACE INTO Hyperedge_Links (hyperedge_id, node_id, type) VALUES (?1, ?2, 'target')",
                params![hyperedge.id.to_string(), target.id.to_string()],
            )?;
        }
    }

    conn.execute("COMMIT", [])?;
    Ok(())
}

fn delete_olog_from_db(olog_id: Uuid) -> Result<(), rusqlite::Error> {
    let conn = Connection::open("olog.db")?;

    // Example DELETE query, adjust according to your schema
    conn.execute(
        "DELETE FROM Ologs WHERE olog_id = ?",
        params![olog_id.to_string()],
    )?;

    Ok(())
}

fn validate_olog_schema(json_data: &str) -> Result<(), serde_json::Error> {
    let _olog: JsonOlogSchema = serde_json::from_str(json_data)?;
    Ok(())
}

fn get_openai_response(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new(env::var("OPENAI_API_KEY")?);

    let req = ChatCompletionRequest::new(
        "mistralai/mixtral-8x7b".to_string(),
        vec![chat_completion::ChatCompletionMessage {
            role: chat_completion::MessageRole::user,
            content: prompt,
            name: None,
            function_call: None,
        }],
    );

    let result = client.chat_completion(req)?;

    // Handling the Option<String> with ok_or
    result
        .choices
        .get(0)
        .and_then(|choice| choice.message.content.clone())
        .ok_or_else(|| "No response from OpenAI".into()) // Converting to Result
}

fn get_openai_response_json(prompt: String) -> Result<String, Box<dyn std::error::Error>> {
    let client = Client::new(env::var("OPENAI_API_KEY")?);

    let response_format_value = serde_json::json!({ "type": "json_object" });

    let req = ChatCompletionRequest::new(
        "mistralai/mixtral-8x7b".to_string(),
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
    result
        .choices
        .get(0)
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
        let uuid = *id_map
            .entry(hyperedge.id.clone())
            .or_insert_with(Uuid::new_v4);
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
        let uuid = *id_map
            .entry(json_node.id.clone())
            .or_insert_with(Uuid::new_v4);
        let node = Node {
            id: uuid,
            label: json_node.label.clone(),
        };
        node_map.insert(uuid, node);
    }

    // Convert nodes to Vec<Node>
    let nodes: Vec<Node> = node_map.values().cloned().collect();

    // Process hyperedges and convert sources and targets to Node instances
    let hyperedges = json_olog
        .hyperedges
        .into_iter()
        .map(|json_hyperedge| {
            let hyperedge_id = *id_map
                .entry(json_hyperedge.id.clone())
                .or_insert_with(Uuid::new_v4);
            let sources = json_hyperedge
                .sources
                .iter()
                .filter_map(|source_id| {
                    id_map
                        .get(source_id)
                        .and_then(|&uuid| node_map.get(&uuid).cloned())
                })
                .collect();
            let targets = json_hyperedge
                .targets
                .iter()
                .filter_map(|target_id| {
                    id_map
                        .get(target_id)
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
        })
        .collect();

    Olog {
        id: Uuid::new_v4(),
        title: json_olog.title,
        nodes,
        hyperedges,
    }
}

fn merge_ologs(olog1: Olog, olog2: Olog) -> Olog {
    let mut node_map = HashMap::new();
    let mut hyperedge_map = HashMap::new();

    // Merge nodes
    for node in olog1.nodes.into_iter().chain(olog2.nodes.into_iter()) {
        node_map.entry(node.label.clone()).or_insert(node);
    }

    // Preparing merged nodes for hyperedge linking
    let merged_nodes = node_map.values().cloned().collect::<Vec<Node>>();

    // Helper to find node by label
    let find_node_by_label = |label: &str| merged_nodes.iter().find(|n| n.label == label).cloned();

    // Merge hyperedges
    for hyperedge in olog1
        .hyperedges
        .into_iter()
        .chain(olog2.hyperedges.into_iter())
    {
        let source_nodes = hyperedge
            .source
            .iter()
            .filter_map(|node| find_node_by_label(&node.label))
            .collect::<Vec<Node>>();
        let target_nodes = hyperedge
            .target
            .iter()
            .filter_map(|node| find_node_by_label(&node.label))
            .collect::<Vec<Node>>();

        // Key for identifying unique hyperedges
        let hyperedge_key = (
            hyperedge.label.clone(),
            source_nodes.clone(),
            target_nodes.clone(),
        );

        hyperedge_map.entry(hyperedge_key).or_insert(Hyperedge {
            id: Uuid::new_v4(), // Assign a new UUID for merged hyperedge
            label: hyperedge.label,
            source: source_nodes,
            target: target_nodes,
            citations: hyperedge.citations,
        });
    }

    Olog {
        id: olog1.id,
        title: olog1.title,
        nodes: merged_nodes,
        hyperedges: hyperedge_map.values().cloned().collect(),
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

async fn ocr_pdf_post(pdf_url: &str, replicate_api_key: &str) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();
    let req_body = OcrPdfPostRequest {
        version: "fbf959aabb306f7cc83e31da4a5ee0ee78406d11216295dbd9ef75aba9b30538".to_string(),
        input: OcrInput {
            document: pdf_url.to_string(),
            postprocess: false,
            early_stopping: false,
        },
    };

    let response = client
        .post("https://api.replicate.com/v1/deployments/chartierluc/nougat/predictions")
        .header("Authorization", format!("Token {}", replicate_api_key))
        .json(&req_body)
        .send()
        .await?;

    let response_body: OcrPdfPostResponse = response.json().await?;
    Ok(response_body.urls.get)
}

async fn ocr_pdf_get(
    prediction_url: &str,
    replicate_api_key: &str,
) -> Result<OcrPdfGetResponse, reqwest::Error> {
    let client = reqwest::Client::new();

    let response = client
        .get(prediction_url)
        .header("Authorization", format!("Token {}", replicate_api_key))
        .send()
        .await?;

    let response_body: OcrPdfGetResponse = response.json().await?;
    Ok(response_body)
}

async fn ocr_pdf_poll(
    prediction_url: String,
    replicate_api_key: String,
) -> Result<String, Box<dyn std::error::Error>> {
    let timeout_duration = Duration::from_secs(240); // 4 minutes

    timeout(timeout_duration, async {
        loop {
            let response = ocr_pdf_get(&prediction_url, &replicate_api_key).await?;

            match response.status.as_str() {
                "succeeded" => return Ok(response.output.ok_or("No output found")?),
                "error" => return Err("Error in processing".into()),
                _ => tokio::time::sleep(tokio::time::Duration::from_secs(1)).await,
            }
        }
    })
    .await?
}

async fn fetch_text_from_url(url: &str) -> Result<String, reqwest::Error> {
    let client = reqwest::Client::new();
    let response = client.get(url).send().await?;

    response.text().await
}

async fn process_paper_and_generate_olog(
    paper_url: &str,
    count: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let replicate_api_key = env::var("REPLICATE_API_TOKEN")
        .map_err(|_| "REPLICATE_API_TOKEN environment variable not set")?;

    let mut merged_olog: Option<Olog> = None;
    let prediction_url = ocr_pdf_post(paper_url, &replicate_api_key).await?;
    let ocr_result_url = ocr_pdf_poll(prediction_url, replicate_api_key.clone()).await?;
    let ocr_result = fetch_text_from_url(&ocr_result_url).await?;

    for _ in 0..count {
        let new_olog = generate_olog(ocr_result.clone())?;

        merged_olog = if let Some(existing_olog) = merged_olog {
            Some(merge_ologs(existing_olog, new_olog))
        } else {
            Some(new_olog)
        };
    }

    if let Some(final_olog) = merged_olog {
        write_olog_to_db(&final_olog)?;
        println!(
            "Merged Olog written to database successfully. UUID: {:?}",
            final_olog.id
        );
    }

    Ok(())
}

fn convert_olog_to_json(olog: &Olog) -> JsonOlogSchema {
    // Convert Nodes
    let json_nodes: Vec<JsonNodeSchema> = olog
        .nodes
        .iter()
        .map(|node| {
            JsonNodeSchema {
                id: node.id.to_string(), // Assuming each node has a unique identifier
                label: node.label.clone(),
            }
        })
        .collect();

    // Convert Hyperedges
    let json_hyperedges: Vec<JsonHyperedgeSchema> = olog
        .hyperedges
        .iter()
        .map(|hyperedge| {
            JsonHyperedgeSchema {
                id: hyperedge.id.to_string(), // Assuming each hyperedge has a unique identifier
                label: hyperedge.label.clone(),
                sources: hyperedge
                    .source
                    .iter()
                    .map(|node| node.id.to_string())
                    .collect(),
                targets: hyperedge
                    .target
                    .iter()
                    .map(|node| node.id.to_string())
                    .collect(),
            }
        })
        .collect();

    // Construct the final JsonOlogSchema
    JsonOlogSchema {
        title: olog.title.clone(), // Assuming the title is a direct copy
        nodes: json_nodes,
        hyperedges: json_hyperedges,
    }
}

fn fetch_and_format_olog_hypergraph(uuid_str: &str) -> Result<String, Box<dyn std::error::Error>> {
    let uuid = Uuid::parse_str(uuid_str)?;

    // Fetch the Olog from the database
    let olog = read_olog_from_db(uuid)?;

    // Convert the Olog to the JSON structure
    let json_olog = convert_olog_to_json(&olog);

    // Serialize to JSON string
    let json_str = serde_json::to_string(&json_olog)?;

    Ok(json_str)
}

fn main() {
    let matches = App::new("Olog Management System")
        .version("1.0")
        .author("Your Name")
        .about("Manages Ologs")
        .subcommand(
            SubCommand::with_name("generate-olog")
                .about("Generates an Olog from a given markdown file")
                .arg(
                    Arg::with_name("FILE")
                        .help("The path to the markdown file")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("merge-ologs")
                .about("Merges two Ologs and updates the database")
                .arg(
                    Arg::with_name("ID1")
                        .help("The ID of the first Olog to merge")
                        .required(true),
                )
                .arg(
                    Arg::with_name("ID2")
                        .help("The ID of the second Olog to merge")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("read-db")
                .about("Reads an Olog from the database")
                .arg(
                    Arg::with_name("ID")
                        .help("The ID of the Olog to read")
                        .required(true),
                ),
        )
        .subcommand(
            SubCommand::with_name("process-paper")
                .about("Processes a paper from a given URL and adds an Olog to the database")
                .arg(
                    Arg::with_name("URL")
                        .help("The URL of the paper to process")
                        .required(true)
                        .takes_value(true),
                )
                .arg(
                    Arg::with_name("COUNT")
                        .help("Number of times to generate an Olog")
                        .required(true)
                        .takes_value(true)
                        .validator(|v| {
                            v.parse::<usize>()
                                .map(|_| ())
                                .map_err(|_| "COUNT must be an integer")
                        }),
                ),
        )
        .subcommand(
            SubCommand::with_name("olog-json")
                .about("Fetches an Olog from the database and outputs its json hypergraph")
                .arg(
                    Arg::with_name("UUID")
                        .help("The UUID of the Olog to fetch")
                        .required(true)
                        .takes_value(true),
                ),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("generate-olog", sub_m)) => {
            let file_path = sub_m.value_of("FILE").unwrap();
            let text = match std::fs::read_to_string(file_path) {
                Ok(content) => content,
                Err(e) => {
                    eprintln!("Failed to read file '{}': {}", file_path, e);
                    return;
                }
            };

            let olog = match generate_olog(text) {
                Ok(olog) => olog,
                Err(e) => {
                    eprintln!("An error occurred in generating Olog: {}", e);
                    return;
                }
            };

            match write_olog_to_db(&olog) {
                Ok(_) => println!("Olog written to database successfully. UUID: {:?}", olog.id),
                Err(e) => eprintln!("Error writing Olog to database: {}", e),
            }
        }
        Some(("merge-ologs", sub_m)) => {
            let id1_str = sub_m.value_of("ID1").unwrap();
            let id2_str = sub_m.value_of("ID2").unwrap();

            let olog1_id = match Uuid::parse_str(id1_str) {
                Ok(uuid) => uuid,
                Err(_) => {
                    eprintln!("Invalid UUID format for ID1");
                    return;
                }
            };

            let olog2_id = match Uuid::parse_str(id2_str) {
                Ok(uuid) => uuid,
                Err(_) => {
                    eprintln!("Invalid UUID format for ID2");
                    return;
                }
            };

            let olog1 = match read_olog_from_db(olog1_id) {
                Ok(olog) => olog,
                Err(e) => {
                    eprintln!("Error reading Olog1 from database: {}", e);
                    return;
                }
            };

            let olog2 = match read_olog_from_db(olog2_id) {
                Ok(olog) => olog,
                Err(e) => {
                    eprintln!("Error reading Olog2 from database: {}", e);
                    return;
                }
            };

            let merged_olog = merge_ologs(olog1, olog2);

            if let Err(e) = delete_olog_from_db(olog1_id) {
                eprintln!("Error deleting Olog1 from database: {}", e);
                return;
            }

            if let Err(e) = delete_olog_from_db(olog2_id) {
                eprintln!("Error deleting Olog2 from database: {}", e);
                return;
            }

            match write_olog_to_db(&merged_olog) {
                Ok(_) => println!(
                    "Merged Olog written to database successfully. UUID: {:?}",
                    merged_olog.id
                ),
                Err(e) => eprintln!("Error writing merged Olog to database: {}", e),
            }
        }
        Some(("read-db", sub_m)) => {
            let id = sub_m.value_of("ID").unwrap().to_string();
            match Uuid::parse_str(&id) {
                Ok(uuid) => match read_olog_from_db(uuid) {
                    Ok(olog) => println!("Olog from DB: {:?}", olog),
                    Err(e) => eprintln!("Error reading from DB: {}", e),
                },
                Err(_) => eprintln!("Invalid UUID format"),
            }
        }
        Some(("process-paper", sub_m)) => {
            let paper_url = sub_m.value_of("URL").unwrap();
            let count = sub_m.value_of("COUNT").unwrap().parse::<usize>().unwrap();

            tokio::runtime::Runtime::new().unwrap().block_on(async {
                if let Err(e) = process_paper_and_generate_olog(paper_url, count).await {
                    eprintln!("Error processing paper: {}", e);
                }
            });
        }
        Some(("olog-json", sub_m)) => {
            let uuid = sub_m.value_of("UUID").unwrap();

            match fetch_and_format_olog_hypergraph(uuid) {
                Ok(json_str) => println!("{}", json_str),
                Err(e) => eprintln!("Error fetching Olog: {}", e),
            }
        }
        _ => eprintln!("Invalid command"),
    }
}
