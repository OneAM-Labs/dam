use std::fs::{self, File};
use std::io;
use std::path::Path;
use flate2::read::GzDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use tar::Archive;

pub fn run(file_path: String) {
    let path = Path::new(&file_path);
    if !path.exists() {
        println!("Error: Cannot locate file '{}'", file_path);
        return;
    }

    if file_path.ends_with(".dam") {
        crate::core::project::import_project(&file_path);
        return;
    }

    // Legacy import code preserved
    if file_path.ends_with(".seal") {
        let tar_gz = File::open(path).unwrap();
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);
        
        let temp_dir = Path::new(".dam/tmp_import");
        fs::create_dir_all(temp_dir).unwrap();
        archive.unpack(temp_dir).unwrap();

        integrate_from_temp(temp_dir, false);
        fs::remove_dir_all(temp_dir).unwrap();
        println!("Successfully imported .seal file.");

    } else if file_path.ends_with(".zip") {
        let file = File::open(path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        
        let temp_dir = Path::new(".dam/tmp_import");
        fs::create_dir_all(temp_dir).unwrap();
        archive.extract(temp_dir).unwrap();

        integrate_from_temp(temp_dir, true);
        fs::remove_dir_all(temp_dir).unwrap();
        println!("Successfully imported raw files from .zip.");
    } else {
        println!("Unsupported format. Please use .seal, .zip, or .dam");
    }
}

// Moves files from extraction temp to the live `.dam` structure
fn integrate_from_temp(temp_dir: &Path, compress_objects: bool) {
    for entry in fs::read_dir(temp_dir).unwrap().flatten() {
        let name = entry.file_name().into_string().unwrap();
        
        if name.ends_with(".json") {
            fs::copy(entry.path(), Path::new(".dam/seals").join(&name)).unwrap();
        } else if name == "objects" && !compress_objects {
            for obj in fs::read_dir(entry.path()).unwrap().flatten() {
                let dest = Path::new(".dam/objects").join(obj.file_name());
                if !dest.exists() {
                    fs::copy(obj.path(), dest).unwrap();
                }
            }
        }
    }

    if compress_objects {
        if let Some(json_file) = fs::read_dir(temp_dir).unwrap().flatten().find(|e| e.file_name().to_string_lossy().ends_with(".json")) {
            let meta_content = fs::read_to_string(json_file.path()).unwrap();
            if let Ok(seal) = serde_json::from_str::<crate::commands::base_commands::seal::Seal>(&meta_content) {
                for file_meta in seal.files {
                    if !file_meta.is_dir {
                        let extracted_raw_file = temp_dir.join(&file_meta.path);
                        if extracted_raw_file.exists() {
                            let dest_obj = Path::new(".dam/objects").join(&file_meta.hash);
                            if !dest_obj.exists() {
                                let mut raw = File::open(extracted_raw_file).unwrap();
                                let compressed_file = File::create(dest_obj).unwrap();
                                let mut encoder = ZlibEncoder::new(compressed_file, Compression::default());
                                io::copy(&mut raw, &mut encoder).unwrap();
                                encoder.finish().unwrap();
                            }
                        }
                    }
                }
            }
        }
    }
}