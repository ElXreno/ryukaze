//! What the window reads of the laptop itself, once, off its thread: what
//! it is, its keyboard's picture and its logo.

use std::sync::Arc;

use crate::{
    bootlogo::{self, Logo},
    layouts::{self, Layout},
    machine::{self, Machine},
};

#[derive(Debug, Clone)]
pub struct Laptop {
    pub machine: Machine,
    pub layout: Arc<Layout>,
    pub logo: Option<Logo>,
}

impl Laptop {
    /// Any laptop: nothing known of it, the default picture.
    pub fn any() -> Arc<Laptop> {
        Arc::new(Laptop {
            machine: Machine::default(),
            layout: Arc::new(layouts::default()),
            logo: None,
        })
    }
}

/// This one, read on a thread of its own; any laptop where that fails.
pub async fn read() -> Arc<Laptop> {
    tokio::task::spawn_blocking(|| {
        let machine = machine::this();
        let layout = Arc::new(layouts::for_machine(&machine));
        Laptop {
            machine,
            layout,
            logo: bootlogo::read(),
        }
    })
    .await
    .map_or_else(|_| Laptop::any(), Arc::new)
}
