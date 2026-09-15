use toukan_robot_core::{paths::AppPaths, rag_index};

fn main() {
    let paths = AppPaths::discover();
    match rag_index::rebuild_rag_index(&paths) {
        Ok(health) => println!("{}\n{}", health.status, health.detail),
        Err(error) => {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    }
}
