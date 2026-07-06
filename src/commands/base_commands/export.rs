use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use flate2::read::ZlibDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use zip::write::FileOptions;
use crate::commands::base_commands::seal::Seal;

pub fn run(target: String, zip_format: bool, is_project: bool, profile: Option<String>) {
    if is_project {
        crate::core::project::export_project(&target, profile);
        return;
    }

    // Legacy Seal Export logic maintained below
    let seal_id = target;
    let seal_meta_path = format!(".dam/seals/{}.json", seal_id);
    if !Path::new(&seal_meta_path).exists() {
        println!("Error: Seal '{}' not found.", seal_id);
        return;
    }

    let meta_content = fs::read_to_string(&seal_meta_path).unwrap();
    let seal: Seal = serde_json::from_str(&meta_content).unwrap();

    if zip_format {
        // Zip Export: Uncompress all objects and zip them as cleartext
        let output_name = format!("{}.zip", seal_id);
        let file = File::create(&output_name).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

        // Include the seal metadata
        zip.start_file(format!("{}.json", seal_id), options).unwrap();
        zip.write_all(meta_content.as_bytes()).unwrap();

        for entry in &seal.files {
            if entry.is_dir {
                zip.add_directory(&entry.path, options).unwrap();
            } else {
                let obj_path = Path::new(".dam/objects").join(&entry.hash);
                if obj_path.exists() {
                    zip.start_file(&entry.path, options).unwrap();
                    let compressed_file = File::open(obj_path).unwrap();
                    let mut decoder = ZlibDecoder::new(compressed_file);
                    io::copy(&mut decoder, &mut zip).unwrap();
                }
            }
        }
        zip.finish().unwrap();
        println!("Exported uncompressed working state to '{}'", output_name);

    } else {
        // Seal Export: Tar.gz archive of the exact CAS objects needed
        let output_name = format!("{}.seal", seal_id);
        let tar_gz = File::create(&output_name).unwrap();
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = tar::Builder::new(enc);

        tar.append_path_with_name(&seal_meta_path, format!("{}.json", seal_id)).unwrap();

        for entry in &seal.files {
            if !entry.is_dir {
                let obj_path = Path::new(".dam/objects").join(&entry.hash);
                if obj_path.exists() {
                    tar.append_path_with_name(&obj_path, format!("objects/{}", entry.hash)).unwrap();
                }
            }
        }
        tar.finish().unwrap();
        println!("Exported highly compressed payload to '{}'", output_name);
    }
}