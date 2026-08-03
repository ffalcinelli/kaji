use tokio::fs;

#[tokio::main]
async fn main() {
    let mut entries = fs::read_dir(".").await.unwrap();
    let mut set = tokio::task::JoinSet::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        set.spawn(async move {
            let _ = entry.file_type().await;
        });
    }
}
