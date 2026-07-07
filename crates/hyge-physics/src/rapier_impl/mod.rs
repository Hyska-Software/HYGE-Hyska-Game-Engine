//! Optional `rapier3d` backend glue.
//!
//! R-070 keeps this module intentionally thin: conversion helpers and module
//! boundaries are present so R-071 can add full world synchronization without
//! changing the public crate layout.

pub mod body;
pub mod char_controller;
pub mod collider;
pub mod island;
pub mod joint;
