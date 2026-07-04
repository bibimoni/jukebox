use anyhow::{Context, Result};
use std::path::Path;
use tantivy::directory::MmapDirectory;
use tantivy::schema::{
    Field, IndexRecordOption, Schema, TextFieldIndexing, TextOptions, STRING, STORED,
};
use tantivy::tokenizer::{LowerCaser, SimpleTokenizer, TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};

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

/// A Tantivy 0.26 [`Tokenizer`] backed by Lindera's morphological analyser.
///
/// `lindera-tantivy` 4.0.0 is built against `tantivy` 0.25 / the 0.6
/// `tantivy-tokenizer-api`, whose `Tokenizer` trait is a *different* trait
/// than the 0.7 one that `tantivy` 0.26 expects. Registering its
/// `LinderaTokenizer` with tantivy 0.26's `TokenizerManager` therefore fails
/// to type-check. This wrapper talks to `lindera` directly and implements
/// tantivy 0.26's own `Tokenizer`/`TokenStream` traits, reproducing the small
/// surface of `lindera-tantivy`'s `LinderaTokenizer` (segment text, byte
/// offsets, position) against the matching API.
#[derive(Clone)]
struct LinderaTokenizer {
    inner: lindera::tokenizer::Tokenizer,
}

impl LinderaTokenizer {
    fn new() -> Result<Self> {
        use lindera::dictionary::load_dictionary;
        use lindera::mode::Mode;
        use lindera::segmenter::Segmenter;

        let dict = load_dictionary("embedded://ipadic").context("loading embedded ipadic")?;
        let segmenter = Segmenter::new(Mode::Normal, dict, None);
        Ok(LinderaTokenizer {
            inner: lindera::tokenizer::Tokenizer::new(segmenter),
        })
    }
}

impl Tokenizer for LinderaTokenizer {
    type TokenStream<'a> = LinderaTokenStream<'a>;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        // Tokenisation failures yield an empty stream rather than panicking;
        // the field simply contributes no tokens for that document.
        let tokens = self.inner.tokenize(text).unwrap_or_default();
        LinderaTokenStream {
            tokens,
            token: Token::default(),
            index: 0,
        }
    }
}

struct LinderaTokenStream<'a> {
    tokens: Vec<lindera::token::Token<'a>>,
    token: Token,
    index: usize,
}

impl<'a> TokenStream for LinderaTokenStream<'a> {
    fn advance(&mut self) -> bool {
        if self.index >= self.tokens.len() {
            return false;
        }
        let t = &self.tokens[self.index];
        self.token.text = t.surface.to_string();
        self.token.offset_from = t.byte_start;
        self.token.offset_to = t.byte_end;
        self.token.position = t.position;
        self.token.position_length = t.position_length;
        self.index += 1;
        true
    }

    fn token(&self) -> &Token {
        &self.token
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.token
    }
}

/// Register the `lindera` (embedded IPADIC) and `lowercase` tokenizers on the
/// index's tokenizer manager. Both `build_index` and `Searcher::open` must
/// call this — Tantivy does not persist tokenizer registrations to disk, and a
/// tokenizer named in the schema must be resolvable at query time too.
fn register_tokenizers(index: &Index) -> Result<()> {
    let lindera = LinderaTokenizer::new()?;
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
        let index = Index::open_or_create(directory, schema)?;
        register_tokenizers(&index)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;
        Ok(Searcher { reader, fields, index })
    }
}
