//! A realistic tagged union, to compare how union encoders are written:
//! a newtype around a struct, a struct-like variant with heap fields, one
//! with plain numbers, a fieldless variant and a newtype around a number.

use girders::prelude::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Login {
    pub user: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub from: String,
    pub to: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Resize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Login(Login),
    Message(Message),
    Resize(Resize),
    Heartbeat,
    Count(u64),
}

fn login() -> impl Codec<Output = Login> {
    record((
        field("user", string(), |l: &Login| &l.user),
        field("at", u64(), |l: &Login| &l.at),
    ))
    .build(|(user, at)| Login { user, at })
}

pub fn event() -> impl Codec<Output = Event> {
    tagged_union(
        "type",
        (
            variant("login", login(), Event::Login),
            variant(
                "message",
                record((
                    field("from", string(), |m: &Message| &m.from),
                    field("to", string(), |m: &Message| &m.to),
                    field("body", string(), |m: &Message| &m.body),
                ))
                .build(|(from, to, body)| Message { from, to, body }),
                Event::Message,
            ),
            variant(
                "resize",
                record((
                    field("width", u32(), |r: &Resize| &r.width),
                    field("height", u32(), |r: &Resize| &r.height),
                ))
                .build(|(width, height)| Resize { width, height }),
                Event::Resize,
            ),
            variant("heartbeat", empty_object(), |()| Event::Heartbeat),
            variant(
                "count",
                record((field("n", u64(), |n: &u64| n),)).build(|(n,)| n),
                Event::Count,
            ),
        ),
        |e| match e {
            Event::Login(login) => Pick5::Variant0(login),
            Event::Message(message) => Pick5::Variant1(message),
            Event::Resize(resize) => Pick5::Variant2(resize),
            Event::Heartbeat => Pick5::Variant3(&()),
            Event::Count(n) => Pick5::Variant4(n),
        },
    )
}

/// 10,000 events, a mix of every variant.
pub fn sample() -> Vec<Event> {
    (0..10_000u64)
        .map(|i| match i % 5 {
            0 => Event::Login(Login {
                user: format!("user{i}"),
                at: i,
            }),
            1 => Event::Message(Message {
                from: format!("user{i}"),
                to: format!("user{}", i + 1),
                body: format!("message number {i}, with some text in it"),
            }),
            2 => Event::Resize(Resize {
                width: i as u32,
                height: (i * 2) as u32,
            }),
            3 => Event::Heartbeat,
            _ => Event::Count(i),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let events = sample();
        let json = girders_json::encode(&events, &list(event())).unwrap();
        assert_eq!(girders_json::decode(&json, &list(event())).unwrap(), events);
    }
}
