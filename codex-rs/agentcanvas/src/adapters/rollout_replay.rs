//! Rollout replay adapter for persisted session files

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use codex_protocol::protocol::{EventMsg, RolloutItem, RolloutLine};
use futures::stream::Stream;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, BufReader};
use tracing::warn;

use crate::schema::*;
use crate::utils::{current_timestamp, generate_item_id, generate_turn_id, parse_iso8601};

use super::{AdapterError, EventAdapter};

/// State for an in-progress item
#[derive(Debug, Clone)]
struct ItemState {
    item_type: ItemType,
    details: ItemDetails,
}

/// Adapter for rollout replay from JSONL files
pub struct RolloutReplayAdapter<R: AsyncBufRead + Unpin> {
    reader: BufReader<R>,
    buffer: String,
    thread_id: Option<ThreadId>,
    turn_id_counter: usize,
    item_id_counter: usize,
    current_turn_id: Option<TurnId>,
    item_states: HashMap<ItemId, ItemState>,
    pending_events: Vec<NormalizedEvent>,
}

impl<R: AsyncBufRead + Unpin> RolloutReplayAdapter<R> {
    /// Create a new rollout replay adapter
    pub fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
            buffer: String::new(),
            thread_id: None,
            turn_id_counter: 0,
            item_id_counter: 0,
            current_turn_id: None,
            item_states: HashMap::new(),
            pending_events: Vec::new(),
        }
    }

    fn process_rollout_item(&mut self, line: RolloutLine) -> Result<Vec<NormalizedEvent>, AdapterError> {
        let mut events = Vec::new();
        let timestamp = parse_iso8601(&line.timestamp).unwrap_or_else(|_| current_timestamp());
        let thread_id = self.thread_id.clone().unwrap_or_default();

        match line.item {
            RolloutItem::SessionMeta(meta) => {
                self.thread_id = Some(meta.session_id.to_string());
                events.push(NormalizedEvent::new(
                    meta.session_id.to_string(),
                    None,
                    None,
                    timestamp,
                    EventSource::RolloutReplay,
                    "SessionMeta".to_string(),
                    EventPayload::ThreadStarted(ThreadStartedPayload {}),
                ));
            }

            RolloutItem::EventMsg(event_msg) => {
                // Map event messages to normalized events
                // TODO: Implement full mapping
                match event_msg {
                    EventMsg::TurnStarted => {
                        let turn_id = generate_turn_id(self.turn_id_counter);
                        self.turn_id_counter += 1;
                        self.current_turn_id = Some(turn_id.clone());

                        events.push(NormalizedEvent::new(
                            thread_id,
                            Some(turn_id),
                            None,
                            timestamp,
                            EventSource::RolloutReplay,
                            "TurnStarted".to_string(),
                            EventPayload::TurnStarted(TurnStartedPayload {}),
                        ));
                    }

                    EventMsg::TurnComplete => {
                        events.push(NormalizedEvent::new(
                            thread_id,
                            self.current_turn_id.clone(),
                            None,
                            timestamp,
                            EventSource::RolloutReplay,
                            "TurnComplete".to_string(),
                            EventPayload::TurnCompleted(TurnCompletedPayload {
                                usage: TokenUsage {
                                    input_tokens: 0,
                                    cached_input_tokens: 0,
                                    output_tokens: 0,
                                },
                            }),
                        ));
                    }

                    _ => {
                        // TODO: Implement other event types
                        warn!("Unhandled event message: {:?}", event_msg);
                    }
                }
            }

            _ => {
                // TODO: Handle other rollout item types
            }
        }

        Ok(events)
    }
}

impl<R: AsyncBufRead + Unpin> Stream for RolloutReplayAdapter<R> {
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

                match serde_json::from_str::<RolloutLine>(line) {
                    Ok(rollout_line) => {
                        match self.process_rollout_item(rollout_line) {
                            Ok(mut events) => {
                                if events.is_empty() {
                                    cx.waker().wake_by_ref();
                                    return Poll::Pending;
                                }

                                let first = events.remove(0);
                                // Store remaining events
                                events.reverse();
                                self.pending_events.extend(events);

                                Poll::Ready(Some(Ok(first)))
                            }
                            Err(e) => {
                                warn!("Error processing rollout item: {}", e);
                                Poll::Ready(Some(Err(e)))
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse rollout line: {} - line: {}", e, line);
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

impl<R: AsyncBufRead + Unpin> EventAdapter for RolloutReplayAdapter<R> {
    fn source(&self) -> EventSource {
        EventSource::RolloutReplay
    }
}
