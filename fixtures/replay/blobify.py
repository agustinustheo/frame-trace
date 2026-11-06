"""Convert a self-contained `--save` directory into fixture form.

The stored metadata is emptied: a replay derives metadata by running the runtime on the
saved proof nodes and uses stored metadata only with --unauthenticated-metadata. Every proof
node of at least THRESHOLD bytes moves into BLOBS/<blake2-256>.zst (zstd -19) and is
replaced by `blob:<hash>`; the artifact is recompressed with zstd -19. Existing blobs with
the same hash are reused.

    python3 -I fixtures/replay/blobify.py <saved-dir> <fixture-dir> fixtures/replay/blobs 100000
"""
import hashlib, json, os, subprocess, sys

src, dst, blobs, threshold = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
raw = subprocess.run(["zstd", "-dc", os.path.join(src, "artifact.json.zst")],
                     check=True, capture_output=True).stdout
artifact = json.loads(raw)


def blob(value):
    if value.startswith("blob:") or len(value) // 2 < threshold:
        return value
    data = bytes.fromhex(value[2:])
    digest = hashlib.blake2b(data, digest_size=32).hexdigest()
    path = os.path.join(blobs, digest + ".zst")
    if not os.path.exists(path):
        subprocess.run(["zstd", "-19", "-q", "-o", path], input=data, check=True)
    return "blob:" + digest


artifact["metadata"] = ""
artifact["proof_nodes"] = [blob(node) for node in artifact["proof_nodes"]]
os.makedirs(dst, exist_ok=True)
out = json.dumps(artifact, separators=(",", ":")).encode()
subprocess.run(["zstd", "-19", "-q", "-f", "-o", os.path.join(dst, "artifact.json.zst")],
               input=out, check=True)
