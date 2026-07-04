use anyhow::{Context, Result};
use std::path::Path;
use tantivy::directory::MmapDirectory;
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, STRING, STORED,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

use lindera::dictionary::load_dictionary;
use lindera::mode::Mode;
use lindera::segmenter::Segmenter;
use lindera_tantivy::tokenizer::LinderaTokenizer;

use crate::catalog::Catalog;
use crate::translit::variants;

/// Directory name used for the on-disk Tantivy index under the catalog root.
pub const INDEX_DIR_NAME: &str = "search-index";

/// Field handles for the search schema. Built once and reused for indexing and querying.
#[derive(Clone)]
pub struct SearchFields {
    pub id: Field,
    pub artists: Field,
    pub title: Field,
    pub title_variants: Field,
    pub artist_variants: Field,
    pub album: Field,
    pub quality: Field,
}

/// Build the schema used by both `build_index` and `Searcher::open`.
///
/// Text fields that hold Japanese text (`artists`, `title`, `album`) use the
/// `lindera` tokenizer (Lindera IPADIC morphological analysis). Cross-script
/// romaji/kana variant fields (`title_variants`, `artist_variants`) use a plain
/// `lowercase` tokenizer, since the variants are already pre-romanised by
/// [`crate::translit::variants`] and only need case-folding for matching.
/// `id` is stored (retrieved on hit) but not tokenised; `quality` is indexed
/// verbatim for exact filtering.
pub fn schema_with_tokenizers() -> (Schema, SearchFields) {
    let lindera_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("lindera")
            .set_index_option(IndexRecordOption::Basic),
    );
    let lower_opts = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer("lowercase")
            .set_index_option(IndexRecordOption::Basic),
    );

    let mut b = Schema::builder();
    let id = b.add_text_field("id", STORED);
    let artists = b.add_text_field("artists", lindera_opts.clone());
    let title = b.add_text_field("title", lindera_opts.clone());
    let title_variants = b.add_text_field("title_variants", lower_opts.clone());
    let artist_variants = b.add_text_field("artist_variants", lower_opts);
    let album = b.add_text_field("album", lindera_opts);
    let quality = b.add_text_field("quality", STRING);
    (
        b.build(),
        SearchFields {
            id,
            artists,
            title,
            title_variants,
            artist_variants,
            album,
            quality,
        },
    )
}

/// Build an embedded-IPADIC `LinderaTokenizer` (the maintained
/// `lindera-tantivy` crate's tokenizer) for registration with Tantivy.
fn build_lindera_tokenizer() -> Result<LinderaTokenizer> {
    let dict = load_dictionary("embedded://ipadic").context("loading embedded ipadic")?;
    let segmenter = Segmenter::new(Mode::Normal, dict, None);
    Ok(LinderaTokenizer::from_segmenter(segmenter))
}

/// Register the `lindera` (embedded IPADIC) and `lowercase` tokenizers on the
/// index's tokenizer manager. Both `build_index` and `Searcher::open` must
/// call this — Tantivy does not persist tokenizer registrations to disk, and a
/// tokenizer named in the schema must be resolvable at query time too.
fn register_tokenizers(index: &Index) -> Result<()> {
    let lindera = build_lindera_tokenizer()?;
    let lowercase = TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(LowerCaser)
        .build();

    let mgr = index.tokenizers();
    mgr.register("lindera", lindera);
    mgr.register("lowercase", lowercase);
    Ok(())
}

/// Build (or rebuild) the Tantivy full-text index for `catalog` at `index_dir`.
///
/// The directory is created if missing. Any existing documents are cleared via
/// `delete_all_documents` + commit before the current catalog is re-indexed, so
/// calling this on an existing index refreshes it in place.
pub fn build_index(catalog: &Catalog, index_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(index_dir)?;

    let (schema, fields) = schema_with_tokenizers();
    let directory = MmapDirectory::open(index_dir)?;
    let index = Index::open_or_create(directory, schema)?;
    register_tokenizers(&index)?;

    let mut writer: IndexWriter = index.writer(50_000_000)?;
    writer.delete_all_documents()?;
    for t in &catalog.tracks {
        let title_variants_str = variants(&t.title).join(" ");
        let artist_variants_str = t
            .artists
            .iter()
            .flat_map(|a| variants(a))
            .collect::<Vec<_>>()
            .join(" ");
        let quality = format!("{}bit-{}Hz", t.bit_depth, t.sample_rate_hz);
        writer.add_document(doc!(
            fields.id => t.id.as_str(),
            fields.artists => t.artists.join(" "),
            fields.title => t.title.as_str(),
            fields.title_variants => title_variants_str.as_str(),
            fields.artist_variants => artist_variants_str.as_str(),
            fields.album => t.album.clone().unwrap_or_default().as_str(),
            fields.quality => quality.as_str(),
        ))?;
    }
    writer.commit()?;
    Ok(())
}

/// Handle for searching an existing on-disk index.
///
/// `search()` is implemented in Task 10; Task 9 only opens the index and
/// re-registers the Lindera/lowercase tokenizers so the schema's named
/// tokenizers resolve at query time.
pub struct Searcher {
    #[allow(dead_code)]
    reader: IndexReader,
    #[allow(dead_code)]
    fields: SearchFields,
    #[allow(dead_code)]
    index: Index,
}

impl Searcher {
    pub fn open(index_dir: &Path) -> Result<Searcher> {
        let (schema, fields) = schema_with_tokenizers();
        let directory = MmapDirectory::open(index_dir)?;
        // Read-side handle: use `open` (not `open_or_create`) so a missing index
        // errors instead of silently yielding an empty searcher — lets the TUI
        // detect a missing index and prompt to run `jukebox index`.
        let index = Index::open(directory)?;
        register_tokenizers(&index)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Searcher { reader, fields, index })
    }
}
