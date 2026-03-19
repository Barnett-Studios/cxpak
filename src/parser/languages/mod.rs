#[cfg(feature = "lang-rust")]
pub mod rust;

#[cfg(feature = "lang-typescript")]
pub mod typescript;

#[cfg(feature = "lang-javascript")]
pub mod javascript;

#[cfg(feature = "lang-python")]
pub mod python;

#[cfg(feature = "lang-java")]
pub mod java;

#[cfg(feature = "lang-go")]
pub mod go;

#[cfg(feature = "lang-c")]
pub mod c;

#[cfg(feature = "lang-cpp")]
pub mod cpp;

#[cfg(feature = "lang-ruby")]
pub mod ruby;

#[cfg(feature = "lang-csharp")]
pub mod csharp;

#[cfg(feature = "lang-swift")]
pub mod swift;

#[cfg(feature = "lang-kotlin")]
pub mod kotlin;

// Tier 1 — new programming languages
#[cfg(feature = "lang-bash")]
pub mod bash;

#[cfg(feature = "lang-php")]
pub mod php;

#[cfg(feature = "lang-dart")]
pub mod dart;

#[cfg(feature = "lang-scala")]
pub mod scala;

#[cfg(feature = "lang-lua")]
pub mod lua;

#[cfg(feature = "lang-elixir")]
pub mod elixir;

#[cfg(feature = "lang-zig")]
pub mod zig;

#[cfg(feature = "lang-haskell")]
pub mod haskell;

#[cfg(feature = "lang-groovy")]
pub mod groovy;

#[cfg(feature = "lang-objc")]
pub mod objc;

#[cfg(feature = "lang-r")]
pub mod r;

#[cfg(feature = "lang-julia")]
pub mod julia;

#[cfg(feature = "lang-ocaml")]
pub mod ocaml;

#[cfg(feature = "lang-matlab")]
pub mod matlab;

// Tier 2 — structural/config languages
#[cfg(feature = "lang-css")]
pub mod css;

#[cfg(feature = "lang-scss")]
pub mod scss;

#[cfg(feature = "lang-markdown")]
pub mod markdown;

#[cfg(feature = "lang-json")]
pub mod json_lang;

#[cfg(feature = "lang-yaml")]
pub mod yaml;

#[cfg(feature = "lang-toml")]
pub mod toml_lang;

#[cfg(feature = "lang-dockerfile")]
pub mod dockerfile;

#[cfg(feature = "lang-hcl")]
pub mod hcl;

#[cfg(feature = "lang-proto")]
pub mod proto;

#[cfg(feature = "lang-svelte")]
pub mod svelte;

#[cfg(feature = "lang-makefile")]
pub mod makefile;

#[cfg(feature = "lang-html")]
pub mod html;

#[cfg(feature = "lang-graphql")]
pub mod graphql;

#[cfg(feature = "lang-xml")]
pub mod xml;
