use anyhow::{Context, Result};
use dotenv::dotenv;
use futures::StreamExt;
use genai::Client as GenAiClient;
use genai::chat::{
    ChatMessage, ChatOptions, ChatRequest, ChatResponseFormat, ChatStream, ChatStreamResponse,
    JsonSpec,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::Instant;

//const AI_MODEL: &'static str = "gemma3:27b-it-qat";
const AI_MODEL: &'static str = "gemini-2.5-flash-preview-04-17";
//const AI_MODEL: &'statiuc str = "gemini-2.0-flash";

/// Basic struct of gen ai output
#[derive(Debug, Clone, Serialize, Deserialize)]
struct GenerativeAIOutput {
    names: Vec<String>,
}

/// Holds parsing context for each block in the structure file
struct ContextEntry {
    key: String,
    indent: usize,
    theme: Option<String>,
    kv_inserts: Vec<String>,
    prefix: Option<String>,
    has_data: bool,
    child_count: usize,
    path: Vec<String>,
}

/// Sanitizes name into a valid localization key fragment
fn sanitize_key(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' | '-' | '\'' | '!' | '"' => '_',
            c if c.is_ascii_alphanumeric() => c.to_ascii_uppercase(),
            _ => '_',
        })
        .collect()
}

/// Helper to call AI and write raw CSV to cache, showing streamed chunks
async fn generate_and_cache(
    client: &GenAiClient,
    cache_path: &Path,
    lore: &str,
    theme: &str,
) -> Result<String> {
    println!("[AI] Streaming generation for theme '{}'", theme);
    let prompt_text = format!(
        r#"
- Prefer to use Latinization of languages (a-z alphabet) and **do not under accents**
- Come up with **as many** possible names
- Avoid duplicates
Come up with as many {} names as possible using the lore:
{}
"#,
        theme, lore
    );
    let user_msg = ChatMessage::user(prompt_text);
    let chat_req = ChatRequest::new(vec![user_msg]);
    let chat_opts = ChatOptions::default()
        .with_temperature(0.5)
        .with_max_tokens(65536)
        .with_response_format(ChatResponseFormat::JsonSpec(JsonSpec::new(
            "names",
            serde_json::json!({
                "type": "object",
                "properties": {
                    "names": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                    }
                }
            }),
        )))
        .with_capture_content(true);

    // Stream the chat
    let stream_response: ChatStreamResponse = client
        .exec_chat_stream(AI_MODEL, chat_req, Some(&chat_opts))
        .await?;
    let mut stream: ChatStream = stream_response.stream;

    let mut combined = String::new();
    println!();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(genai::chat::ChatStreamEvent::Start) => {}
            Ok(genai::chat::ChatStreamEvent::Chunk(stream_chunk)) => {
                print!("{}", stream_chunk.content);
                combined.push_str(&stream_chunk.content);
            }
            Ok(genai::chat::ChatStreamEvent::ReasoningChunk(stream_chunk)) => {
                print!("{}", stream_chunk.content);
            }
            Ok(genai::chat::ChatStreamEvent::End(end)) => {
                println!("Final out: {:?}", end.captured_content);
                break;
            }
            Err(e) => {
                eprintln!("[AI Warning] Streaming error: {}", e);
                break;
            }
        }
    }
    println!();

    // gracefully close off the json if not complete
    // remove trailing ,
    if let Some(last_quote_pos) = combined.rfind('"') {
        let mut idx = last_quote_pos + 1;
        // Skip whitespace
        while idx < combined.len() && combined.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        // If next character is a comma, remove it
        if idx < combined.len() && combined.as_bytes()[idx] == b',' {
            combined.remove(idx);
        }
    }

    // Fix common JSON issues in-place
    let mut fixed = combined.clone();
    // Keep content starting at first '{'
    if let Some(pos) = fixed.find('{') {
        fixed = fixed[pos..].to_string();
    }
    // Ensure quotes are balanced
    if fixed.matches('"').count() % 2 != 0 {
        fixed.push('"');
    }
    // Remove empty trailing string entries (incomplete " element)
    {
        let trimmed = fixed.trim_end();
        // if ends with two quotes indicating an empty string
        if trimmed.ends_with("\"\"") {
            // drop the empty "" and any leading comma
            if let Some(pos) = fixed.rfind(",\"\"") {
                fixed.replace_range(pos..pos + 3, "");
            }
        }
    }
    // Remove trailing comma after last quoted string
    if let Some(last_q) = fixed.rfind('"') {
        let mut idx = last_q + 1;
        while idx < fixed.len() && fixed.as_bytes()[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx < fixed.len() && fixed.as_bytes()[idx] == b',' {
            fixed.remove(idx);
        }
    }
    // Balance brackets and braces
    let ob = fixed.matches('[').count();
    let cb = fixed.matches(']').count();
    if cb < ob {
        fixed.push_str(&"]".repeat(ob - cb));
    }
    let obc = fixed.matches('{').count();
    let cbc = fixed.matches('}').count();
    if cbc < obc {
        fixed.push_str(&"}".repeat(obc - cbc));
    }

    // Write cache
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).ok();
    }
    fs::write(&cache_path, &combined).context("Failed to write cache file")?;
    println!(
        "[AI] Cached {} bytes to '{}'",
        combined.len(),
        cache_path.display()
    );
    Ok(combined)
}

/// Generates or reads cached raw CSV of names, then applies prefix formatting.
async fn generate_localized_entries(
    client: &GenAiClient,
    cache_path: &Path,
    lore: &str,
    theme: &str,
    prefix: &str,
) -> Result<Vec<(String, String)>> {
    let raw = if let Ok(string) = fs::read_to_string(&cache_path) {
        if !string.trim().is_empty() {
            println!(
                "[Cache] '{}' exists—using cached names",
                cache_path.display()
            );
            string
        } else {
            generate_and_cache(client, cache_path, lore, theme).await?
        }
    } else {
        generate_and_cache(client, cache_path, lore, theme).await?
    };
    let mut json_out: Option<GenerativeAIOutput> = serde_json::from_str(&raw)
        .map_err(|e| println!("[Gen AI Error]: {}", e))
        .ok();
    // keep trying over and over
    while json_out.is_none() {
        json_out =
            serde_json::from_str(&generate_and_cache(client, cache_path, lore, theme).await?)
                .map_err(|e| println!("[Gen AI Error]: {}", e))
                .ok();
    }
    let json_out: GenerativeAIOutput = json_out.unwrap();
    let prefix_clean = prefix.trim_end_matches('_');
    let mut entries = Vec::new();
    for nm in json_out.names {
        let name = nm.trim();
        if name.is_empty() {
            continue;
        }
        let nm_san = sanitize_key(name);
        let key = if prefix_clean.is_empty() {
            nm_san.clone()
        } else {
            format!("{}_{}", prefix_clean, nm_san)
        };
        entries.push((key, name.to_string()));
    }
    Ok(entries)
}

#[tokio::main]
async fn main() -> Result<()> {
    let start = Instant::now();
    dotenv().ok();
    println!("[Start] Initializing generation process");
    
    fs::create_dir_all("cache").context("Failed to create cache dir")?;

    let lore = fs::read_to_string("lore.txt").context("Failed to read lore.txt")?;
    let structure =
        fs::read_to_string("file_structure.txt").context("Failed to read file_structure.txt")?;

    let client = GenAiClient::default();
    let mut stack: Vec<ContextEntry> = Vec::new();
    let mut pending_theme: Option<String> = None;
    let mut pending_kvs: Vec<String> = Vec::new();
    let mut pending_prefix: Option<String> = None;
    let mut output: Vec<String> = Vec::new();
    let mut localisations: HashMap<String, String> = HashMap::new();

    for raw_line in structure.lines() {
        let indent = raw_line.chars().take_while(|c| c.is_whitespace()).count();
        let trimmed = raw_line.trim();

        if trimmed.starts_with('#') {
            let comment = &trimmed[1..].trim();
            if let Some((k, v)) = comment.split_once('=') {
                pending_kvs.push(format!("{} = {}", k.trim(), v.trim()));
            } else if let Some(pref) = comment.strip_prefix("prefix:") {
                pending_prefix = Some(pref.trim().to_string());
            } else {
                pending_theme = Some(comment.to_string());
            }
            continue;
        }

        if trimmed.ends_with('{') {
            let key = trimmed
                .split_once('=')
                .map(|(a, _)| a.trim())
                .unwrap_or(trimmed)
                .to_string();
            let mut path = if let Some(parent) = stack.last() {
                parent.path.clone()
            } else {
                Vec::new()
            };
            path.push(key.clone());
            let cur_prefix = pending_prefix
                .take()
                .or_else(|| stack.last().and_then(|p| p.prefix.clone()));
            let ctx = ContextEntry {
                key: key.clone(),
                indent,
                theme: pending_theme.take(),
                kv_inserts: pending_kvs.clone(),
                prefix: cur_prefix,
                has_data: false,
                child_count: 0,
                path,
            };
            pending_kvs.clear();

            output.push(raw_line.to_string());
            for kv in &ctx.kv_inserts {
                let kv_indent = " ".repeat(indent + 4);
                output.push(format!("{}{}", kv_indent, kv));
            }
            stack.push(ctx);
            continue;
        }

        if trimmed == "}" {
            if let Some(ctx) = stack.pop() {
                if ctx.child_count == 0 && !ctx.has_data && ctx.theme.is_some() {
                    let theme = ctx.theme.unwrap();
                    let prefix = ctx.prefix.clone().unwrap_or_default();
                    let filename = ctx.path.join("_");
                    let cache_file = Path::new("cache").join(format!("{}.txt", filename));
                    let entries =
                        generate_localized_entries(&client, &cache_file, &lore, &theme, &prefix)
                            .await?;
                    for (key, val) in entries {
                        output.push(format!("{}{},", " ".repeat(ctx.indent + 4), key));
                        localisations.entry(key.clone()).or_insert(val);
                    }
                }
            }
            output.push(raw_line.to_string());
            if let Some(parent) = stack.last_mut() {
                parent.child_count += 1;
                parent.has_data = true;
            }
            continue;
        }

        output.push(raw_line.to_string());
        if let Some(ctx) = stack.last_mut() {
            if trimmed.contains('=') || trimmed.contains(',') {
                ctx.has_data = true;
            }
        }
    }

    fs::write("out.txt", output.join("\n")).context("Failed to write out.txt")?;
    let mut loc_out = String::from("l_english:\n");
    for (key, val) in &localisations {
        loc_out.push_str(&format!("    {}:0 \"{}\"\n", key, val));
    }
    fs::write("localisation.txt", loc_out).context("Failed to write localisation.txt")?;

    println!("Completed in {:.2?}", start.elapsed());
    Ok(())
}
