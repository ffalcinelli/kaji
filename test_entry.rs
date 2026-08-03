#[tokio::main]
async fn main() {
    let mut entries = tokio::fs::read_dir(".").await.unwrap();
    let mut set = tokio::task::JoinSet::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        set.spawn(async move {
            let path = entry.path();
            let file_type = entry.file_type().await.unwrap();
            println!("{:?} {:?}", path, file_type);
        });
    }
    while let Some(res) = set.join_next().await {
        res.unwrap();
    }
}
