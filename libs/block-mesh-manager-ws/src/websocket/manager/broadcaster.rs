use block_mesh_common::interfaces::ws_api::WsServerMessage;
use dashmap::DashMap;
use futures::future::join_all;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::broadcast::error::SendError;
use tokio::sync::{broadcast, mpsc, Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Broadcaster {
    pub global_transmitter: broadcast::Sender<WsServerMessage>,
    pub sockets: Arc<DashMap<(Uuid, String), mpsc::Sender<WsServerMessage>>>,
    pub queue: Arc<Mutex<VecDeque<(Uuid, String)>>>,
}

impl Default for Broadcaster {
    fn default() -> Self {
        Self::new()
    }
}

impl Broadcaster {
    pub fn new() -> Self {
        let (global_transmitter, _) = broadcast::channel(10000);
        Self {
            global_transmitter,
            sockets: Arc::new(DashMap::new()),
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }
    pub fn broadcast(&self, message: WsServerMessage) -> Result<usize, SendError<WsServerMessage>> {
        let subscribers = self.global_transmitter.send(message.clone())?;
        tracing::info!("Sent {message:?} to {subscribers} subscribers");
        Ok(subscribers)
    }

    pub async fn batch(&self, message: WsServerMessage, targets: &[(Uuid, String)]) {
        join_all(targets.iter().filter_map(|target| {
            if let Some(entry) = self.sockets.get(target) {
                let sink_tx = entry.value().clone();
                let msg = message.clone();
                let future = async move {
                    if let Err(error) = sink_tx.send(msg).await {
                        tracing::warn!("Batch broadcast failed {error:?}");
                    }
                };
                Some(future)
            } else {
                None
            }
        }))
        .await;
    }

    pub async fn move_queue(&self, count: usize) -> Vec<(Uuid, String)> {
        let queue = &mut self.queue.lock().await;
        let count = count.min(queue.len());
        let drained: Vec<(Uuid, String)> = queue.drain(0..count).collect();
        queue.extend(drained.clone());
        drained
    }

    pub async fn broadcast_to_user(
        &self,
        messages: impl IntoIterator<Item = WsServerMessage> + Clone,
        id: (Uuid, String),
    ) {
        let entry = self.sockets.get(&id);
        let msgs = messages.clone();
        if let Some(entry) = entry {
            let tx = entry.value().clone();
            for msg in msgs {
                if let Err(error) = tx.send(msg).await {
                    tracing::warn!("Error while queuing WS message: {error}");
                }
            }
        }
    }

    /// returns a number of nodes to which [`WsServerMessage`]s were sent
    pub async fn queue_multiple(
        &self,
        messages: impl IntoIterator<Item = WsServerMessage> + Clone,
        count: usize,
    ) -> Vec<(Uuid, String)> {
        let drained = self.move_queue(count).await;
        join_all(drained.clone().into_iter().filter_map(|id| {
            if let Some(entry) = self.sockets.get(&id) {
                let tx = entry.value().clone();
                let msgs = messages.clone();
                Some(async move {
                    for msg in msgs {
                        if let Err(error) = tx.send(msg).await {
                            tracing::warn!("Error while queuing WS message: {error}");
                        }
                    }
                })
            } else {
                None
            }
        }))
        .await;
        drained
    }

    pub async fn subscribe(
        &self,
        user_id: Uuid,
        ip: String,
        sink_sender: mpsc::Sender<WsServerMessage>,
    ) -> broadcast::Receiver<WsServerMessage> {
        let _ = self
            .sockets
            .insert((user_id, ip.clone()), sink_sender.clone());
        let queue = &mut self.queue.lock().await;
        queue.push_back((user_id, ip));
        self.global_transmitter.subscribe()
    }

    pub async fn unsubscribe(&self, user_id: Uuid, ip: String) {
        self.sockets.remove(&(user_id, ip.clone()));
        let queue = &mut self.queue.lock().await;
        if let Some(pos) = queue.iter().position(|(a, b)| a == &user_id && b == &ip) {
            queue.remove(pos);
        } else {
            tracing::warn!("Failed to remove a socket from the queue");
        }
    }
}
