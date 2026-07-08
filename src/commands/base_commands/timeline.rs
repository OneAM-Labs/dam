use std::fs;
use std::path::Path;
use std::collections::{HashMap};
use crate::commands::seal::Seal;
use crate::commands::stream::get_or_create_meta;

pub fn run(graph_flag: bool) {
    if !Path::new(".dam").exists() {
        println!("No reservoir found.");
        return;
    }

    if graph_flag {
        render_ascii_dag();
        return;
    }

    let current_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();
    let stream_meta = get_or_create_meta(&current_stream);
    
    let mut current_seal_id = stream_meta.latest_seal;

    println!(" Reservoir Timeline History ({})", current_stream);
    println!("============================================");

    if current_seal_id.is_none() {
        println!("(No history recorded in this stream yet)");
        return;
    }

    // Traverse utilizing parent pointers to guarantee exact historic relationships
    while let Some(seal_id) = current_seal_id {
        let path = format!(".dam/seals/{}.json", seal_id);
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                println!("\n[{}] - {}", seal.id, seal.timestamp);
                
                if !seal.parents.is_empty() {
                    println!("  Parents: {}", seal.parents.join(", "));
                }
                println!("  Stream: {}", seal.stream);
                println!("  Message: {}", seal.message);
                
                let file_paths: Vec<String> = seal.files.iter().map(|f| f.path.clone()).collect();
                println!("  Tracked Files: {}", file_paths.join(", "));
                println!("--------------------------------------------");
                
                // Move backward down the continuity chain
                current_seal_id = seal.parents.get(0).cloned();
            } else {
                println!("Error parsing seal: {}", seal_id);
                break;
            }
        } else {
            println!("Warning: Broken continuity chain. Seal file missing: {}", path);
            break;
        }
    }
}

// Dynamic ASCII DAG visualizer rendering branches and merge relationships beautifully
fn render_ascii_dag() {
    let seals_dir = Path::new(".dam/seals");
    if !seals_dir.exists() {
        println!("No seals exist yet.");
        return;
    }

    // Load all seals
    let mut seals_map = HashMap::new();
    let mut seals_list = Vec::new();

    for entry in fs::read_dir(seals_dir).unwrap().flatten() {
        if entry.path().extension().map_or(false, |ext| ext == "json") {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if let Ok(seal) = serde_json::from_str::<Seal>(&content) {
                    seals_map.insert(seal.id.clone(), seal.clone());
                    seals_list.push(seal);
                }
            }
        }
    }

    // Sort chronologically
    seals_list.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));

    if seals_list.is_empty() {
        println!("No timeline records to display.");
        return;
    }

    println!("\n📊 Reservoir Continuity Graph (Topological Timeline)");
    println!("===================================================");

    // Keep track of virtual tracks/columns representing different streams
    let mut streams_order: Vec<String> = Vec::new();
    let mut node_columns: HashMap<String, usize> = HashMap::new();

    for seal in &seals_list {
        if !streams_order.contains(&seal.stream) {
            streams_order.push(seal.stream.clone());
        }
        let col = streams_order.iter().position(|s| s == &seal.stream).unwrap();
        node_columns.insert(seal.id.clone(), col);
    }

    let active_stream = fs::read_to_string(".dam/CURRENT").unwrap_or_else(|_| "main".to_string()).trim().to_string();

    // Render nodes backward (most recent on top)
    seals_list.reverse();

    for (index, seal) in seals_list.iter().enumerate() {
        let col_idx = *node_columns.get(&seal.id).unwrap();
        
        // Draw connections/edges to parents if applicable
        if index > 0 {
            let mut connection_line = String::new();
            for i in 0..streams_order.len() {
                if i == col_idx {
                    if seal.parents.len() > 1 {
                        connection_line.push_str("⏽ ╲ "); // Draw merge split representation
                    } else {
                        connection_line.push_str("│   ");
                    }
                } else {
                    connection_line.push_str("│   ");
                }
            }
            println!("{}", connection_line.trim_end());
        }

        // Draw node structure
        let mut node_line = String::new();
        for i in 0..streams_order.len() {
            if i == col_idx {
                let sym = if seal.stream == active_stream { "●" } else { "○" };
                node_line.push_str(&format!("{}   ", sym));
            } else {
                node_line.push_str("│   ");
            }
        }

        let clean_time = seal.timestamp.replace("T", " ").split('.').next().unwrap_or(&seal.timestamp).to_string();
        let branch_indicator = format!("[{}]", seal.stream);
        println!(
            "{} {} ({}) - {} \"{}\"",
            node_line.trim_end(),
            seal.id,
            clean_time,
            branch_indicator,
            seal.message
        );
    }
    
    println!("\nLegend:  ● Active Stream Head   ○ Non-Active Stream Head");
    println!();
}