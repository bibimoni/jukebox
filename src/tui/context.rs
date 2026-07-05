//! The play-context abstraction: anything you pick a track from is a `Context`.
//!
//! A `Context` is the source of the "current list" of tracks the user is
//! browsing/playing: an album, an artist, a playlist, a search result, or the
//! queue. Album/Artist/Search carry their track ids inline; Playlist/Queue
//! resolve their ids at runtime via [`ContextResolver`] (implemented by `App`
//! in Task 5) so they always reflect live app state.

use std::collections::BTreeMap;

use crate::catalog::Catalog;

/// Resolves [`Context`] variants that point at live app state (playlists, queue).
/// `App` (Task 5) implements this; tests use a fake.
pub trait ContextResolver {
    fn playlist_ids(&self, name: &str) -> Vec<String>;
    fn queue_ids(&self) -> Vec<String>;
}

/// A single album grouped under an artist: its title, owning artist, and the
/// indices (into the catalog `tracks` slice) of its tracks.
#[derive(Clone)]
pub struct Album {
    pub title: String,
    pub artist: String,
    pub track_indices: Vec<usize>,
}

/// The source of the current track list. Anything you pick a track from is a
/// `Context`.
#[derive(Clone)]
pub enum Context {
    Album {
        album: String,
        artist: String,
        track_ids: Vec<String>,
    },
    Artist {
        artist: String,
        track_ids: Vec<String>,
    },
    Playlist {
        name: String,
    },
    Search {
        query: String,
        track_ids: Vec<String>,
    },
    Queue,
}

impl Context {
    /// Human-readable label for the context bar / pane header.
    pub fn label(&self) -> String {
        match self {
            Context::Album { album, artist, .. } => format!("{artist} — {album}"),
            Context::Artist { artist, .. } => artist.clone(),
            Context::Playlist { name } => format!("♫ {name}"),
            Context::Search { query, .. } => format!("search: {query}"),
            Context::Queue => "Queue".into(),
        }
    }

    /// The track ids this context currently points at. Album/Artist/Search
    /// return their inline ids; Playlist/Queue delegate to the resolver so they
    /// reflect live app state.
    pub fn track_ids(&self, r: &dyn ContextResolver) -> Vec<String> {
        match self {
            Context::Album { track_ids, .. }
            | Context::Artist { track_ids, .. }
            | Context::Search { track_ids, .. } => track_ids.clone(),
            Context::Playlist { name } => r.playlist_ids(name),
            Context::Queue => r.queue_ids(),
        }
    }

    /// Static length of `track_ids` for variants that carry ids inline
    /// (Album/Artist/Search). Playlist/Queue resolve lazily via a
    /// [`ContextResolver`], so they return 0 here; `Transport` recomputes the
    /// real length on demand once a resolver is available.
    pub fn track_ids_placeholder_len(&self) -> usize {
        match self {
            Context::Album { track_ids, .. }
            | Context::Artist { track_ids, .. }
            | Context::Search { track_ids, .. } => track_ids.len(),
            Context::Playlist { .. } | Context::Queue => 0,
        }
    }
}

/// Group catalog tracks into albums per artist, preserving `(disc, track)` order
/// within each album and sorting albums by lowercase title.
///
/// Albums are keyed by `t.album` (falling back to `"(no album)"`) under
/// `t.primary_artist`. Within an album, tracks are sorted by
/// `(disc_number.unwrap_or(1), track_number.unwrap_or(0))`.
pub fn build_albums_by_artist(cat: &Catalog) -> BTreeMap<String, Vec<Album>> {
    let mut map: BTreeMap<String, Vec<Album>> = BTreeMap::new();
    for (i, t) in cat.tracks.iter().enumerate() {
        let artist = t.primary_artist.clone();
        let album = t.album.clone().unwrap_or_else(|| "(no album)".into());
        let entry = map.entry(artist.clone()).or_default();
        if let Some(a) = entry.iter_mut().find(|a| a.title == album) {
            a.track_indices.push(i);
        } else {
            entry.push(Album {
                title: album,
                artist,
                track_indices: vec![i],
            });
        }
    }
    // sort each album's tracks by (disc, track_number), then albums by title
    for albums in map.values_mut() {
        for a in albums.iter_mut() {
            a.track_indices.sort_by_key(|&i| {
                let t = &cat.tracks[i];
                (t.disc_number.unwrap_or(1), t.track_number.unwrap_or(0))
            });
        }
        albums.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    }
    map
}
