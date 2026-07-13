//! Recommendation engine: listening events, candidate generation, ranking,
//! diversity, identity, validation, mixes, radio, feedback, generator,
//! explanations, evaluation.
//! Built on local processing (Level 2) with optional ytmusicapi enrichment
//! (Level 3).

pub mod candidates;
pub mod diversity;
pub mod evaluation;
pub mod events;
pub mod explanations;
pub mod feedback;
pub mod generator;
pub mod identity;
pub mod mixes;
pub mod profile;
pub mod radio;
pub mod ranking;
pub mod validation;
