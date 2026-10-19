import pytest

from jumbo_worker.blobs import BlobError, BlobStore, hash_bytes


@pytest.fixture
def store(tmp_path):
    return BlobStore(tmp_path)


def test_chunked_upload_resumes_and_verifies(store):
    data = bytes(range(256)) * 1000
    name = hash_bytes(data)
    assert store.missing([name]) == [name]
    assert store.write_chunk(name, 0, len(data), data[:100_000]).received == 100_000
    # A client reconnecting asks where to resume.
    assert store.status(name).received == 100_000
    done = store.write_chunk(name, 100_000, len(data), data[100_000:])
    assert done.complete and store.path(name).read_bytes() == data
    assert store.missing([name]) == []


def test_rejects_out_of_order_chunks(store):
    data = b"x" * 10
    with pytest.raises(BlobError, match="expected offset 0"):
        store.write_chunk(hash_bytes(data), 5, 10, data[5:])


def test_discards_corrupt_uploads(store):
    name = hash_bytes(b"real")
    with pytest.raises(BlobError, match="does not match"):
        store.write_chunk(name, 0, 4, b"fake")
    assert store.status(name).received == 0 and not store.has(name)


def test_rejects_bad_names_and_overlong_chunks(store):
    with pytest.raises(BlobError):
        store.status("../etc/passwd")
    with pytest.raises(BlobError, match="past the declared total"):
        store.write_chunk(hash_bytes(b"ab"), 0, 1, b"ab")


def test_put_is_idempotent(store):
    assert store.put(b"out") == store.put(b"out") == hash_bytes(b"out")
