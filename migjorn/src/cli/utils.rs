use crate::Model;

pub fn load_model(path: &std::path::Path) -> Result<Model, String> {
    let timer = std::time::Instant::now();
    let result =
        Model::from_file(path).map_err(|e| format!("Error reading {}: {e}", path.display()));
    println!(
        "Loaded model from {} in {:?}",
        path.display(),
        timer.elapsed()
    );
    result
}
