# Fuzz Artifacts

Each target gets `fuzz/artifacts/<target>/`.

These directories stay in git only through their `README.md` so the structure is stable and discoverable. New crashes generated locally or in CI should land here first, then be minimized and promoted into `fuzz/corpus/<target>/`.
