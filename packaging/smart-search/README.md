# Pinned inference runtime

Strata owns discovery, extraction, SQLite/FTS, ranking, preferences and GTK UI.
Only DigUp's headless `digup worker` JSON-lines inference process is reused;
the DigUp desktop shell and index are not embedded.

`runtime.json` pins DigUp 0.7.0-beta.3 at
`0943225c2690c0631c01041e5dcb77c53e04395e`, archive sizes and SHA-256 digests.
`scripts/smart_search_runtime.py` verifies archives, extracts only the worker,
native libraries and third-party notices with 7-Zip, and records staged digests.
It never runs the upstream installer. Windows release packaging stages this into
`bin/smart-search` before writing its manifest. Models are not bundled.

Development staging:

```powershell
python scripts/smart_search_runtime.py --platform windows --destination target/embedding-runtime
```

For Linux, use `--platform linux`. Installed workers belong beside the executable
in `smart-search/`, or in `lib/strata/smart-search/` / `libexec/strata/smart-search/`.
Ordinary application builds do not download model weights.

Inference uses two CPU threads and separate query/index lifetimes, bounded JSON
and timeouts. Windows uses kill-on-close Job Objects with process/memory limits
and UI restrictions. Job Objects alone are not filesystem/network security
boundaries. Linux additionally requires Bubblewrap with private network/PID
namespaces and read-only runtime/model/artifact mounts; isolation failure leaves
keyword search available.

Models: [EmbeddingGemma 2 GGUF](https://huggingface.co/ggml-org/embeddinggemma-2-GGUF/tree/bfcd298762cc34d0357ece5ebdd31791a3a374d8),
Apache-2.0. Filenames, sizes, digests and fingerprint are pinned in
`src/services/smart_search/model.rs`. Explicit setup verifies hashes before
publishing a compatibility marker. Inference does not access the network.
Changing model, dimension, quantization, segmenter or runtime requires a new
fingerprint and re-embedding incompatible rows.

DigUp is MIT licensed; packaged dependencies retain its
`THIRD-PARTY-NOTICES.txt`. See [notices](../../THIRD_PARTY_LICENSES.md).
This runtime does not change GTK/GStreamer or apply the private media patch kit.
