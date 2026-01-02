//! Messaging-related commands.

use std::time::Duration;

use meshcore::event::{Event, EventFilter};
use meshcore::protocol::PacketType;

use super::{CommandContext, current_timestamp};
use crate::error::{CliError, Result};

impl CommandContext {
    /// Executes the `msg` command.
    pub async fn cmd_msg(
        &self,
        name: &str,
        message: &[String],
        wait: bool,
        timeout_secs: u64,
    ) -> Result<()> {
        let contact = self.get_contact(name).await?;
        let text = message.join(" ");
        let timestamp = current_timestamp();

        let event = self
            .commands()
            .await
            .send_message(&contact.public_key, &text, 0, timestamp)
            .await?;

        match event {
            Event::MessageSent {
                expected_ack,
                timeout_ms,
            } => {
                self.display.print_msg_sent(expected_ack, timeout_ms);
                // Store expected ACK for wait_ack
                let mut state = self.state.lock().await;
                state.last_sender = Some(contact.name.clone());
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        // Wait for ACK if requested
        if wait {
            self.cmd_wait_ack(timeout_secs).await?;
        }

        Ok(())
    }

    /// Executes the `wait_ack` command.
    pub async fn cmd_wait_ack(&self, timeout_secs: u64) -> Result<()> {
        let filter = EventFilter::packet_types(vec![PacketType::Ack]);
        let timeout = Duration::from_secs(timeout_secs);

        match self.wait_for_event(filter, timeout).await {
            Ok(Event::Ack(ack)) => {
                self.display.print_ack(ack.code);
            }
            Ok(_) => {
                return Err(CliError::Timeout("ACK".into()));
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }

    /// Executes the `chan` command.
    pub async fn cmd_chan(&self, channel: u8, message: &[String]) -> Result<()> {
        let text = message.join(" ");
        let timestamp = current_timestamp();

        let event = self
            .commands()
            .await
            .send_channel_message(channel, &text, timestamp)
            .await?;

        match event {
            Event::Ok => {
                self.display.print_ok("channel message sent");
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `public` command (send to channel 0).
    pub async fn cmd_public(&self, message: &[String]) -> Result<()> {
        self.cmd_chan(0, message).await
    }

    /// Executes the `recv` command.
    pub async fn cmd_recv(&self) -> Result<()> {
        let event = self.commands().await.get_message().await?;

        match event {
            Event::ContactMessage(msg) => {
                // Find sender name
                let contacts = self.client.lock().await.contacts().await;
                let sender_name = super::lookup_sender_name(&contacts, &msg.sender_prefix);

                self.display.print_message(
                    &sender_name,
                    &msg.text,
                    msg.text_type == meshcore::types::TextType::Command,
                    msg.signal.as_ref().map(|s| s.snr),
                    None, // v3 format doesn't include RSSI
                );

                // Update last sender
                let mut state = self.state.lock().await;
                state.last_sender = Some(sender_name);
            }
            Event::ChannelMessage(msg) => {
                // Channel messages don't include sender information
                let channel_str = format!("#{}", msg.channel_index);
                self.display.print_message(
                    &channel_str,
                    &msg.text,
                    false,
                    msg.signal.as_ref().map(|s| s.snr),
                    None, // v3 format doesn't include RSSI
                );
            }
            Event::NoMoreMessages => {
                self.display.print_no_more_messages();
            }
            Event::Error { message } => {
                return Err(CliError::Command(message));
            }
            _ => {}
        }

        Ok(())
    }

    /// Executes the `wait_msg` command.
    pub async fn cmd_wait_msg(&self, timeout_secs: u64) -> Result<()> {
        let filter = EventFilter::packet_types(vec![
            PacketType::ContactMsgRecv,
            PacketType::ContactMsgRecvV3,
            PacketType::ChannelMsgRecv,
            PacketType::ChannelMsgRecvV3,
        ]);
        let timeout = Duration::from_secs(timeout_secs);

        match self.wait_for_event(filter, timeout).await {
            Ok(event) => {
                self.handle_message_event(event).await?;
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }

    /// Executes the `trywait_msg` command.
    /// Waits for `MESSAGES_WAITING` event, then reads the message.
    pub async fn cmd_trywait_msg(&self, timeout_secs: u64) -> Result<()> {
        let filter = EventFilter::packet_types(vec![PacketType::MessagesWaiting]);
        let timeout = Duration::from_secs(timeout_secs);

        if self.wait_for_event(filter, timeout).await.is_ok() {
            // Messages are waiting, read them
            let event = self.commands().await.get_message().await?;

            match event {
                Event::ContactMessage(msg) => {
                    let contacts = self.client.lock().await.contacts().await;
                    let sender_name = super::lookup_sender_name(&contacts, &msg.sender_prefix);

                    self.display.print_message(
                        &sender_name,
                        &msg.text,
                        msg.text_type == meshcore::types::TextType::Command,
                        msg.signal.as_ref().map(|s| s.snr),
                        None,
                    );

                    let mut state = self.state.lock().await;
                    state.last_sender = Some(sender_name);
                }
                Event::ChannelMessage(msg) => {
                    let channel_str = format!("#{}", msg.channel_index);
                    self.display.print_message(
                        &channel_str,
                        &msg.text,
                        false,
                        msg.signal.as_ref().map(|s| s.snr),
                        None,
                    );
                }
                Event::NoMoreMessages => {
                    self.display.print_no_more_messages();
                }
                Event::Error { message } => {
                    return Err(CliError::Command(message));
                }
                _ => {}
            }
        }
        // Timeout - no message arrived, that's OK for trywait

        Ok(())
    }

    /// Executes the `sync_msgs` command.
    pub async fn cmd_sync_msgs(&self) -> Result<()> {
        loop {
            let event = self.commands().await.get_message().await?;

            match event {
                Event::ContactMessage(msg) => {
                    let contacts = self.client.lock().await.contacts().await;
                    let sender_name = super::lookup_sender_name(&contacts, &msg.sender_prefix);

                    self.display.print_message(
                        &sender_name,
                        &msg.text,
                        msg.text_type == meshcore::types::TextType::Command,
                        msg.signal.as_ref().map(|s| s.snr),
                        None,
                    );
                }
                Event::ChannelMessage(msg) => {
                    let channel_str = format!("#{}", msg.channel_index);
                    self.display.print_message(
                        &channel_str,
                        &msg.text,
                        false,
                        msg.signal.as_ref().map(|s| s.snr),
                        None,
                    );
                }
                Event::NoMoreMessages => {
                    break;
                }
                Event::Error { message } => {
                    return Err(CliError::Command(message));
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Executes the `msgs_subscribe` command.
    pub async fn cmd_msgs_subscribe(&self) -> Result<()> {
        let mut subscription = self.subscribe().await;

        println!("Subscribed to messages. Press Ctrl+C to stop.");

        loop {
            tokio::select! {
                event = subscription.recv() => {
                    match event {
                        Some(event) => {
                            self.handle_message_event(event).await?;
                        }
                        None => break,
                    }
                }
                _ = tokio::signal::ctrl_c() => {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Handles a message event.
    async fn handle_message_event(&self, event: Event) -> Result<()> {
        match event {
            Event::ContactMessage(msg) => {
                let contacts = self.client.lock().await.contacts().await;
                let sender_name = super::lookup_sender_name(&contacts, &msg.sender_prefix);

                self.display.print_message(
                    &sender_name,
                    &msg.text,
                    msg.text_type == meshcore::types::TextType::Command,
                    msg.signal.as_ref().map(|s| s.snr),
                    None,
                );

                let mut state = self.state.lock().await;
                state.last_sender = Some(sender_name);
            }
            Event::ChannelMessage(msg) => {
                let channel_str = format!("#{}", msg.channel_index);
                self.display.print_message(
                    &channel_str,
                    &msg.text,
                    false,
                    msg.signal.as_ref().map(|s| s.snr),
                    None,
                );
            }
            Event::Ack(ack) => {
                self.display.print_ack(ack.code);
            }
            Event::Advertisement(key) => {
                if !self.display.is_json() {
                    println!("Advertisement from: {}", key.to_hex());
                }

                // Add to pending if manual_add_contacts is enabled
                let mut state = self.state.lock().await;
                state.add_pending(key.to_hex(), None);
            }
            Event::NewContactAdvert(contact) => {
                if !self.display.is_json() {
                    println!(
                        "New contact: {} ({})",
                        contact.name,
                        contact.public_key.to_hex()
                    );
                }

                let mut state = self.state.lock().await;
                state.add_pending(contact.public_key.to_hex(), Some(contact.name.clone()));
            }
            Event::LoginSuccess => {
                self.display.print_ok("Login success");
            }
            Event::LoginFailed => {
                self.display.print_error("Login failed");
            }
            Event::MessagesWaiting => {
                if !self.display.is_json() {
                    println!("Messages waiting on device");
                }
            }
            _ => {}
        }

        Ok(())
    }
}
