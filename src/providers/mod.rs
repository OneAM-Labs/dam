pub mod flutter;
pub mod firebase;
pub mod python;
pub mod custom;


pub trait Provider {
    fn name(&self) -> &'static str;
    fn check_environment(&self) -> bool;
    fn default_toml(&self, project_name: &str) -> String;
}

pub fn get_provider(name: &str) -> Box<dyn Provider> {
    match name.to_lowercase().as_str() {
        "flutter" => Box::new(flutter::FlutterProvider),
        "firebase" => Box::new(firebase::FirebaseProvider),
        "python" => Box::new(python::PythonProvider),
        _ => Box::new(custom::CustomProvider),
    }
}