//! Input scripts for `pshb-cli replay` and end-to-end tests: the same engine
//! and link as with real devices, but the events come from a text file.
//!
//! ```text
//! # comment
//! combo Ctrl+Alt+P        # press in order, release in reverse
//! wait 50                 # milliseconds
//! key LEFTSHIFT down
//! tap A
//! key LEFTSHIFT up
//! move 10 -5
//! wheel 1
//! hwheel -1
//! button BTN_LEFT down
//! focus on
//! ```

use std::time::Duration;

use crate::engine::{Event, REL_HWHEEL, REL_WHEEL, REL_X, REL_Y};
use crate::keymap;
use crate::trigger::Combo;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub delay: Duration,
    pub event: Event,
}

pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut delay = Duration::ZERO;
    let mut push = |delay: &mut Duration, event: Event| {
        steps.push(Step {
            delay: std::mem::take(delay),
            event,
        });
    };
    for (no, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        let err = |msg: &str| format!("line {}: {msg}: \"{line}\"", no + 1);
        let words: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| -> Result<i32, String> {
            words
                .get(i)
                .ok_or_else(|| err("number missing"))?
                .parse()
                .map_err(|_| err("not a number"))
        };
        let code = |i: usize| -> Result<u16, String> {
            let name = words.get(i).ok_or_else(|| err("key missing"))?;
            keymap::code_by_name(name).ok_or_else(|| err("unknown key"))
        };
        match words[0] {
            "wait" => delay += Duration::from_millis(num(1)?.max(0) as u64),
            "key" | "button" => {
                let down = match words.get(2) {
                    Some(&"down") => true,
                    Some(&"up") => false,
                    _ => return Err(err("expected \"down\" or \"up\"")),
                };
                push(&mut delay, Event::Key { code: code(1)?, down });
                push(&mut delay, Event::Sync);
            }
            "tap" => {
                let c = code(1)?;
                for down in [true, false] {
                    push(&mut delay, Event::Key { code: c, down });
                    push(&mut delay, Event::Sync);
                }
            }
            "combo" => {
                let combo = Combo::parse(words.get(1).ok_or_else(|| err("Kombination fehlt"))?)?;
                let codes: Vec<u16> = combo.0.iter().map(|alt| alt.0[0]).collect();
                for &c in &codes {
                    push(&mut delay, Event::Key { code: c, down: true });
                    push(&mut delay, Event::Sync);
                }
                for &c in codes.iter().rev() {
                    push(&mut delay, Event::Key { code: c, down: false });
                    push(&mut delay, Event::Sync);
                }
            }
            "move" => {
                push(
                    &mut delay,
                    Event::Rel {
                        axis: REL_X,
                        value: num(1)?,
                    },
                );
                push(
                    &mut delay,
                    Event::Rel {
                        axis: REL_Y,
                        value: num(2)?,
                    },
                );
                push(&mut delay, Event::Sync);
            }
            "wheel" | "hwheel" => {
                let axis = if words[0] == "wheel" {
                    REL_WHEEL
                } else {
                    REL_HWHEEL
                };
                push(&mut delay, Event::Rel { axis, value: num(1)? });
                push(&mut delay, Event::Sync);
            }
            "focus" => match words.get(1) {
                Some(&"on") => push(&mut delay, Event::Focus(true)),
                Some(&"off") => push(&mut delay, Event::Focus(false)),
                _ => return Err(err("expected \"on\" or \"off\"")),
            },
            _ => return Err(err("unknown command")),
        }
    }
    Ok(steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_commands() {
        let s =
            parse("# test\ncombo Ctrl+P\nwait 20\ntap A\nmove 3 -4\nwheel 1\nkey LEFTSHIFT down\nfocus on\n")
                .unwrap();
        assert_eq!(s[0].event, Event::Key { code: 29, down: true });
        let tap = s
            .iter()
            .position(|x| x.event == Event::Key { code: 30, down: true })
            .unwrap();
        assert_eq!(s[tap].delay, Duration::from_millis(20));
        assert!(s.contains(&Step {
            delay: Duration::ZERO,
            event: Event::Rel {
                axis: REL_Y,
                value: -4
            }
        }));
        assert_eq!(s.last().unwrap().event, Event::Focus(true));
    }

    #[test]
    fn errors_name_the_line() {
        assert!(parse("tap A\nkey Q sideways").unwrap_err().starts_with("line 2"));
        assert!(parse("jump").is_err());
    }
}
