//! MIDI keyboard input. The driver callback only forwards note messages and
//! wakes the UI; the editor owns the audition voice as for the on-screen piano.
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender, channel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MidiEvent {
    On { note: u8, velocity: u8 },
    Off { note: u8 },
    AllOff,
}
/// Any channel; running status is resolved by the driver.
pub(crate) fn parse(message: &[u8]) -> Option<MidiEvent> {
    let (&status, data) = message.split_first()?;
    match (status & 0xf0, data) {
        (0x90, &[note, velocity, ..]) if velocity > 0 => Some(MidiEvent::On {
            note: note & 0x7f,
            velocity: velocity & 0x7f,
        }),
        (0x80 | 0x90, &[note, ..]) => Some(MidiEvent::Off { note: note & 0x7f }),
        // All Sound Off / All Notes Off.
        (0xb0, &[120 | 123, ..]) => Some(MidiEvent::AllOff),
        _ => None,
    }
}

pub(crate) struct Midi {
    connection: Option<midir::MidiInputConnection<()>>,
    pub port: Option<String>,
    sender: Sender<MidiEvent>,
    events: Receiver<MidiEvent>,
}
impl Default for Midi {
    fn default() -> Self {
        let (sender, events) = channel();
        Self {
            connection: None,
            port: None,
            sender,
            events,
        }
    }
}
impl Midi {
    pub fn ports() -> Result<Vec<String>, String> {
        let input = midir::MidiInput::new("DiscoFM").map_err(|e| e.to_string())?;
        Ok(input
            .ports()
            .iter()
            .filter_map(|port| input.port_name(port).ok())
            .collect())
    }
    pub fn connect(&mut self, name: &str, ctx: egui::Context) -> Result<(), String> {
        self.disconnect();
        let input = midir::MidiInput::new("DiscoFM").map_err(|e| e.to_string())?;
        let port = input
            .ports()
            .into_iter()
            .find(|port| input.port_name(port).is_ok_and(|n| n == name))
            .ok_or_else(|| format!("MIDI input \"{name}\" is no longer available"))?;
        let sender = self.sender.clone();
        let connection = input
            .connect(
                &port,
                "discofm-input",
                move |_, message, _| {
                    if let Some(event) = parse(message)
                        && sender.send(event).is_ok()
                    {
                        ctx.request_repaint();
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        self.connection = Some(connection);
        self.port = Some(name.to_owned());
        discofm_log::info("midi", format!("listening to {name}"));
        Ok(())
    }
    pub fn disconnect(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
        }
        self.port = None;
        while self.events.try_recv().is_ok() {}
    }
    pub fn drain(&self) -> Vec<MidiEvent> {
        self.events.try_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_messages_on_any_channel() {
        assert_eq!(
            parse(&[0x93, 60, 100]),
            Some(MidiEvent::On {
                note: 60,
                velocity: 100
            })
        );
        assert_eq!(parse(&[0x90, 60, 0]), Some(MidiEvent::Off { note: 60 }));
        assert_eq!(parse(&[0x8f, 61, 64]), Some(MidiEvent::Off { note: 61 }));
        assert_eq!(parse(&[0xb0, 123, 0]), Some(MidiEvent::AllOff));
        assert_eq!(parse(&[0xb0, 7, 100]), None);
        assert_eq!(parse(&[0xf8]), None);
        assert_eq!(parse(&[0x90]), None);
        assert_eq!(parse(&[]), None);
    }
}
