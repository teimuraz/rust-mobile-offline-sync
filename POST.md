<!--
Title (put this in the platform's TITLE field — dev.to / Medium / Hashnode render
it as the page H1, so it must NOT also appear as a `#` heading in the body):

I built an offline-first sync engine with event sourcing, in Rust shared between mobile and backend
-->

*A small, honest write-up of a thing I built mostly for fun, that somehow ended up working. Code: [github.com/teimuraz/rust-mobile-offline-sync](https://github.com/teimuraz/rust-mobile-offline-sync)*

First, the setup — because it explains all the Rust in this post. The app I built this for is [TrainVision](https://trainvision.ai), a mobile-first platform for collecting machine-learning training data out in the real world (where reliable connectivity is often exactly what you don't have). Its **core is written in Rust and shared across iOS and Android** via [UniFFI](https://mozilla.github.io/uniffi-rs/): the models, the sync, the business rules are written *once* in Rust and compiled into a native library both phones call through generated Swift and Kotlin bindings. The **backend is Rust too**, so the same code runs there. Storage is **SQLite on the device** and **Postgres on the backend**. That shared-Rust core is half of what this post is about; event sourcing is the other half — and the two turn out to fit together beautifully for offline sync.

Now the actual problem. I needed an app that works with no internet. Not "degrades gracefully" — genuinely works: you're in a field, a factory, a basement, you capture data all day, and it syncs whenever a connection comes back. Sometimes there's good signal, often it's weak, sometimes there's none at all — the point of offline-*first* is that the app never assumes the network is there. Connectivity is a nice bonus when it shows up, not something the app leans on.

I looked for something existing first — Couchbase (Lite + Sync Gateway), ObjectBox, a few others. I got furthest with Couchbase, but couldn't get it to click. The moment I wanted to control how conflicts resolved or shape sync around my own domain, I felt like I was fighting the framework instead of using it — and I'd be paying for the privilege. *[add your own one-liner about the specific wall you hit]*

I don't recommend "just build your own sync engine" as career advice. But I was curious, it looked fun, and I wanted full control. So I started sketching how I'd do it myself — and it hit me: **event sourcing.**

It wasn't a bolt from the blue. I'd played with event sourcing before, in a completely different context — a classic event-sourced *backend* architecture, nothing to do with offline or mobile. I'd found it a genuinely interesting way to build a system: state as a fold over an append-only log of domain events. So when I started thinking about how two offline devices could ever reconcile, that old idea walked right back in the door wearing a new hat.

## Why event sourcing makes offline sync easy

The hard part of offline-first isn't storing data locally. It's *merging*. Two devices edit the same thing while both are offline; someone deletes a record another person just updated; the same change syncs twice on a flaky connection. If each device only keeps the **current state**, merging means shipping whole entities back and forth, comparing them field by field, and guessing what changed — and every one of those guesses is a chance to get it wrong. I wanted something stupidly simple instead.

Event sourcing flips it. You don't store the item. You store the **events** that produced it:

```
ItemCreated      { name: "Wrench", quantity: 10 }
QuantityChanged  { quantity: 8 }
NoteChanged      { note: "left bin" }
```

The current item is just a *fold* over those events — replay them in order and you get the current state. (That folded result has a name in event-sourcing circles: a **projection**.) That one shift changes everything for sync, because events are:

- **Append-only** — devices never overwrite each other, they only add. Sync becomes "ship the events the other side hasn't seen yet."
- **Idempotent to replay** — every event has an id, so receiving it twice is a no-op. A dropped connection mid-sync costs nothing but a retry.
- **Orderable** — put every replica's events into one agreed order and they all fold to the same state.

My entire entity contract is one small trait:

```rust
pub trait EventSourcedEntity: Default {
    type Evt: Event;
    type EntId: EntityId;

    /// The only thing a domain type must define: given the current state and one
    /// event, produce the next state.
    fn apply_event(&mut self, event: Self::Evt, modification_info: ModificationInfo);

    fn uncommitted_events(&mut self) -> &mut Vec<EntityEvent<Self::Evt>>;
    fn id(&self) -> &Self::EntId;

    /// Rebuild by folding history (already in canonical order).
    fn build_from_history(events: Vec<EventDescriptor<Self::Evt, Self::EntId>>) -> Self {
        let mut entity = Self::default();
        for event in events {
            let info = event.modification_info();
            entity.apply_event(event.payload, info);
        }
        entity
    }

    /// Record a new local change: remember it as uncommitted, and apply it now so
    /// the UI updates immediately — even fully offline.
    fn new_event(&mut self, event: EntityEvent<Self::Evt>) {
        self.uncommitted_events().push(event.clone());
        self.apply_event(event.event, event.modification_info);
    }
}
```

A domain type implements `apply_event` **once** — "given current me and this event, here's the new me" — and gets local optimistic updates, history rebuild, and sync for free. In the demo repo that domain type is a deliberately boring inventory `Item`, and its `apply_event` is the whole thing:

```rust
fn apply_event(&mut self, event: ItemEvent, info: ModificationInfo) {
    if self.deleted {
        return; // deletion is terminal — this line is the entire "delete wins" rule
    }
    self.last_modified_ms = info.replica_time_ms;
    match event {
        ItemEvent::Created { name, quantity } => { self.name = name; self.quantity = quantity; }
        ItemEvent::Renamed { name }           => self.name = name,
        ItemEvent::QuantityChanged { quantity } => self.quantity = quantity,
        ItemEvent::NoteChanged { note }       => self.note = note,
        ItemEvent::Deleted                    => self.deleted = true,
    }
}
```

## "Why not just sync the database change log?"

Fair question — the obvious alternative is row-level change-data-capture: ship the database's change log (WAL / logical replication / a `changes` table) and replay it on the other side. Plenty of sync systems work exactly this way.

Honestly, I don't have a rigorous answer for why I didn't. It came down to this: **I didn't want to sync rows, I wanted to sync intent.** A row change tells you `quantity: 10 → 8`. A domain event tells you *`QuantityChanged { 8 }`* — the actual thing that happened, with the meaning attached. That difference ended up mattering for two things I cared about: resolving conflicts (a `Deleted` event can *mean* "this beats a concurrent edit", where two raw row-writes just look like a clash), and — the bigger one — **authorization**.

## Authorization: who can push and pull what

This turned out to be a big one for me. In my app, a user is **not** allowed to sync everything. They can read and write only a slice of the data: some **common/shared data** everyone in their scope sees, plus **their own data** — and nothing else. So when a device pushes a batch of events, the server has to check, for each event: *is this user allowed to write this?* And when it pulls: *which data is this user even allowed to see?*

Domain events gave me a natural place to enforce that. Every event knows which entity it targets and who authored it, so the server can authorize each pushed event against the user's permitted scope, and filter the pull side down to exactly the slices they're allowed to read. Maybe you can do this just as well with a row-log approach — I genuinely didn't dig into it. This just felt like a clean seam to me, so I went with it.

*(The demo repo keeps this part deliberately simple — the point there is the sync mechanics — but scoped, per-event authorization is a first-class concern in the real app, not an afterthought.)*

## The honest part: ordering across devices

Here's where I stop pretending it's perfect. To fold events deterministically you need a **total order across devices**, and there is no global clock. My rule:

> Order by the event's **replica (device) time** first. Device clocks drift, and can even be set wrong on purpose — *this is a fact we accept.* If two events land on the exact same timestamp, a per-device counter breaks the tie; if even that matches, the event id does.

When I poked at how some established apps handle this — Apple Notes, for one — they seem to lean on device local time in much the same way. That made me feel a lot better about not over-thinking it.

```rust
pub struct ModificationInfo {
    pub replica_id: String,     // which device/server produced it
    pub replica_time_ms: i64,   // that replica's wall-clock at creation
    pub write_offset: u64,      // per-replica tie-breaker
}

// the canonical fold order, applied on every replica before folding:
fn order_key(d: &EventDescriptor) -> (i64, u64, Uuid) {
    (d.replica_time_ms, d.write_offset, d.event_id)
}
```

So the whole rule is: **the most recent change by the device clock wins**, and everything gets applied in that order. It's not a clever academic algorithm — I deliberately kept it dead simple, predictable, and easy to reason about. The one extra rule I add: **deletion wins over concurrent edits** — and as you saw above, that's not special machinery, it's a single `if self.deleted { return; }`.

Sync itself is then almost boring, which is the goal. The server stamps each event with a monotonic `ServerOffset`. A device remembers the highest offset it has pulled and asks: *"give me everything after this."* It folds the new events into its projections, ships its own un-synced events up, done. In the real app that ask-and-ship is a gRPC round-trip to the backend; in the demo repo it's just an in-process function call — same logic, different transport. No diffing, no merge UI, no magic:

```rust
pub trait EventLog<E, EntId> {
    fn append(&mut self, events: Vec<AppendEvent<E, EntId>>) -> Vec<EventDescriptor<E, EntId>>;
    fn events_of_entity(&self, entity_id: &EntId) -> Vec<EventDescriptor<E, EntId>>;
    fn events_after(&self, offset: Option<ServerOffset>, limit: usize) -> EventsPage<E, EntId>;
}
```

## The part I'm actually proud of: one codebase, three platforms

I write Rust for the backend. The mobile app is iOS (Swift) and Android (Kotlin). And it isn't just the sync plumbing that's shared — the **domain models and their events are shared too.** The `Item` entity, the `ItemEvent` enum that spells out every change that can happen to it, the fold that turns those events into current state, the ordering, the codecs both sides must agree on byte-for-byte — all of it is the **exact same Rust code** running in all three places. I define an entity and its events *once*, and the backend and both phones already agree, down to the byte, on what they are.

The device and the server implement the *same* `EventLog` trait. On the backend it's backed by **Postgres**; on the phone by **SQLite**. Same interface, same folding logic, same conflict rules — because it is *literally the same functions* on top of two different storage backends. On mobile the crate is compiled to a native library and exposed to Swift/Kotlin through UniFFI, so iOS and Android share it too.

## Does it actually work? Here's it running

The repo ships the real thing, end to end: an axum backend with its own event log, and a SwiftUI app driving the shared Rust SDK — local edits, background sync job, and an **Online toggle** that simulates losing the network.

```sh
cargo run -p backend --bin server   # the backend (in-memory; Postgres in reality)
./mobile/ios/build_rust.sh          # compile the Rust SDK for iOS + Swift bindings
```

Then run the app in **two simulators side by side** — each install gets its own replica id, so they're two independent devices syncing through the real server over HTTP:

- Add items on device A → they show up on device B a few seconds later.
- Toggle device B offline, edit the *same* item on both, bring B back online → both devices converge (later device clock wins), and flipping back online triggers an immediate sync.
- Delete an item on A while B edits it offline → the deletion wins everywhere.

And you can watch it from the server's side while you do it:

```sh
curl localhost:4000/items       # current state, folded server-side from the events
curl localhost:4000/events/all  # the raw event log with all the provenance
```

The fold, two-replica convergence, and deletion-wins are also pinned down as unit tests (`cargo test -p shared`).

## It's simple — and it works

I'll be clear-eyed: this is a small, deliberately simple design, not a big framework, and there are edge cases I've chosen to accept rather than solve. But the clock-based ordering has held up fine in practice — and, as I mentioned, it seems to be roughly how apps like Apple Notes handle it too. It survives real offline use, and I understand every line — which, after fighting a black-box sync framework, is worth a lot.

If it's useful to anyone, I pulled the core idea into a tiny, generic demo — an offline-first inventory of `Item`s — with the shared Rust crate, the sync engine, and the runnable simulation above:

**→ [github.com/teimuraz/rust-mobile-offline-sync](https://github.com/teimuraz/rust-mobile-offline-sync)**

It's deliberately small. The point isn't the app; it's the shape: one Rust event model, folded on the server and on the phone, synced by a plain offset cursor. If you're staring down offline-first and the existing tools feel like too much, maybe event sourcing is your cheat code too.

This is the engine behind [TrainVision](https://trainvision.ai) — if you're collecting training data in the field and want it to just work offline, that's what it's for.

*Built with Rust + UniFFI. Questions and roasts welcome — in the comments or on [GitHub](https://github.com/teimuraz/rust-mobile-offline-sync/issues).*

*— Teimuraz, building [TrainVision](https://trainvision.ai)*
