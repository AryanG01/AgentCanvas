//! App-server v2 adapter for JSON-RPC notifications

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::stream::Stream;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tracing::warn;

use crate::schema::*;
use crate::utils::current_timestamp;

use super::{AdapterError, EventAdapter};

/// State for an in-progress item
#[derive(Debug, Clone)]
struct ItemState {
    item_type: ItemType,
    details: ItemDetails,
}

/// Adapter for app-server v2 JSON-RPC notifications
pub struct AppServerV2Adapter<R: AsyncBufRead + Unpin> {
    reader: BufReader<R>,
    buffer: String,
    thread_id: Option<ThreadId>,
    turn_id: Option<TurnId>,
    item_states: HashMap<ItemId, ItemState>,
    pending_events: Vec<NormalizedEvent>,
}

impl<R: AsyncBufRead + Unpin> AppServerV2Adapter<R> {
    /// Create a new app-server v2 adapter
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: String::new(),
            thread_id: None,
            turn_id: None,
            item_states: HashMap::new(),
            pending_events: Vec::new(),
        }
    }

    fn process_notification(&mut self, _notification: serde_json::Value) -> Result<Vec<NormalizedEvent>, AdapterError> {
        // TODO: Implement full notification processing
        Ok(Vec::new())
    }
}

impl<R: AsyncBufRead + Unpin> Stream for AppServerV2Adapter<R> {
    type Item = Result<NormalizedEvent, AdapterError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // Check for pending events first
        if let Some(event) = self.pending_events.pop() {
            return Poll::Ready(Some(Ok(event)));
        }

        // Read next line
        self.buffer.clear();
        match futures::ready!(Pin::new(&mut self.reader).poll_read_line(cx, &mut self.buffer)) {
            Ok(0) => Poll::Ready(None), // EOF
            Ok(_) => {
                let line = self.buffer.trim();
                if line.is_empty() {
                    cx.waker().wake_by_ref();
                    return Poll::Pending;
                }

                match serde_json::from_str::<serde_json::Value>(line) {
                    Ok(notification) => {
                        match self.process_notification(notification) {
                            Ok(mut events) => {
                                if events.is_empty() {
                                    cx.waker().wake_by_ref();
                                    return Poll::Pending;
                                }

                                let first = events.remove(0);
                                events.reverse();
                                self.pending_events.extend(events);

                                Poll::Ready(Some(Ok(first)))
                            }
                            Err(e) => {
                                warn!("Error processing notification: {}", e);
                                Poll::Ready(Some(Err(e)))
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse notification: {} - line: {}", e, line);
                        Poll::Ready(Some(Err(AdapterError::ParseError {
                            message: e.to_string(),
                            raw_input: line.to_string(),
                        })))
                    }
                }
            }
            Err(e) => Poll::Ready(Some(Err(AdapterError::IoError(e)))),
        }
    }
}

impl<R: AsyncBufRead + Unpin> EventAdapter for AppServerV2Adapter<R> {
    fn source(&self) -> EventSource {
        EventSource::AppServerV2
    }
}
