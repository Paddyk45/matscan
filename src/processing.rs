pub mod minecraft;
pub mod minecraft_fingerprinting;

use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddrV4,
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use parking_lot::Mutex;

use crate::{
    config::Config,
    database::{self, bulk_write::CollectionExt, Database},
};

pub struct SharedData {
    pub database: Database,
    /// The queue of servers to process, along with their server list ping
    /// response.
    pub queue: VecDeque<(SocketAddrV4, Vec<u8>)>,
    /// Data from the previous scan, used for identifying players that just
    /// joined or left a server.
    pub cached_servers: HashMap<SocketAddrV4, serde_json::Value>,

    pub total_new: usize,
    pub revived: usize,
    pub results: usize,

    /// The number of tasks that are actively processing server responses.
    pub processing_count: usize,
}

#[async_trait]
pub trait ProcessableProtocol: Send + 'static {
    fn process(
        shared: &Arc<Mutex<SharedData>>,
        config: &Config,
        target: SocketAddrV4,
        data: &[u8],
        database: &Database,
    ) -> Option<database::bulk_write::BulkUpdate>;
}

/// A task that processes pings from the queue.
pub async fn process_pings<P>(shared: Arc<Mutex<SharedData>>, config: Config)
where
    P: ProcessableProtocol + Send + 'static,
{
    let database = shared.lock().database.clone();
    loop {
        if shared.lock().queue.is_empty() {
            // wait a bit until next loop
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }

        shared.lock().processing_count += 1;

        let mut bulk_updates: Vec<database::bulk_write::BulkUpdate> = Vec::new();
        let updating = shared.lock().queue.drain(..100).collect::<Vec<_>>();
        for (target, data) in updating {
            bulk_updates.push(P::process(&shared, &config, target, &data, &database));
        }

        let mongo_db = database.mcscanner_database();
        mongo_db
            .collection::<bson::Document>("servers")
            .bulk_update(&mongo_db, bulk_updates)
            .await;

        shared.lock().processing_count -= 1;
        println!("processing count is now {}", shared.lock().processing_count);
    }
}
