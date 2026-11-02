- **Faster CI on pull requests.** The wasm jobs install a prebuilt wasm-bindgen CLI instead of building it
  from source, `web-app` and `web-parity` share one Rust cache, CI builds keep line tables instead of full
  debug info, and pull requests no longer save their own caches. Main's caches should then fit in the
  10 GB limit, so a pull request restores them instead of building cold.
