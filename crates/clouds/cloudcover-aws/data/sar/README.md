# Pinned AWS Service Authorization Reference

Normal Cargo builds validate `catalog.json` against the SHA-256 digest in
`manifest.json` and compile it without network access or a shared generated cache.
The manifest records the index digest, retrieval time, extractor revision and
refresh script digest. Each service retains its upstream URL, modification marker
and original response digest. The catalog digest pins the normalized documents.

Refresh explicitly from the repository root with `python3 tools/refresh_sar.py`.
Review and commit both changed files together. The updater checks that the index
has not changed during acquisition; AWS does not offer an atomic historical SAR
snapshot. Review policy and FAS tests before accepting an update. The FAS supplement
remains independently versioned in `../fas.json`.

Migration: `CLOUDCOVER_AWS_SAR_REFRESH` and the old shared `actions-v2.rs` cache no
longer select inputs. The checked-in catalog is authoritative. No SDK histories or
public Rust lookup APIs change. Offline builds still need locally available Cargo
and Go dependencies; the SAR acquisition itself is no longer part of a build.
