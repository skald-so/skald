# A local S3-compatible server in a container for the tests of attachment storage in a bucket:
# versitygw, its buckets and objects in a tmpfs. Sourced by script/test, script/mutants and
# script/image-test; the caller removes the container with s3_stop, e.g. in its EXIT trap.

s3_image=versity/versitygw:v1.11.0@sha256:8319a530d08d537426e2f855bae5280709795dc73a3f0c347973ef915d6343c5
# Not S3's default region: a client that ignores the region setting is refused.
s3_region=eu-central-2
s3_access_key_id=skald-test-access-key
s3_secret_key=skald-test-secret-key-6f1d0c2e9a4b7358
# The port it listens on in its container.
s3_port=7070

# s3_start [<docker run option>...]: starts the container with the options, such as another port to
# publish, and sets $s3 to its ID. It listens on $s3_port, published on a loopback port.
s3_start() {
  s3=$(docker run --detach --rm --tmpfs /data --tmpfs /meta --publish "127.0.0.1::$s3_port" "$@" \
    "$s3_image" --access "$s3_access_key_id" --secret "$s3_secret_key" --region "$s3_region" \
    --port ":$s3_port" posix --sidecar /meta /data)
}

# s3_wait: waits until it answers, then exports its URL, region and credentials as
# SKALD_TEST_S3_URL, SKALD_TEST_S3_REGION, SKALD_TEST_S3_ACCESS_KEY_ID and SKALD_TEST_S3_SECRET_KEY.
s3_wait() {
  SKALD_TEST_S3_URL=http://$(docker port "$s3" "$s3_port/tcp" | head -n 1)
  for _ in $(seq 120); do
    [[ $(curl --silent --output /dev/null --write-out '%{http_code}' "$SKALD_TEST_S3_URL/") == 000 ]] ||
      break
    sleep 0.5
  done
  if [[ $(curl --silent --output /dev/null --write-out '%{http_code}' "$SKALD_TEST_S3_URL/") == 000 ]]; then
    echo "script/${0##*/}: the S3-compatible server did not start" >&2
    docker logs "$s3" >&2
    exit 1
  fi
  SKALD_TEST_S3_REGION=$s3_region
  SKALD_TEST_S3_ACCESS_KEY_ID=$s3_access_key_id
  SKALD_TEST_S3_SECRET_KEY=$s3_secret_key
  export SKALD_TEST_S3_URL SKALD_TEST_S3_REGION SKALD_TEST_S3_ACCESS_KEY_ID SKALD_TEST_S3_SECRET_KEY
}

# s3_curl <curl argument>...: curl with a request signed by its credentials.
s3_curl() {
  curl --silent --show-error --aws-sigv4 "aws:amz:$s3_region:s3" \
    --user "$s3_access_key_id:$s3_secret_key" "$@"
}

# s3_create_bucket <name>: creates the bucket <name> in its region.
s3_create_bucket() {
  s3_curl --fail --output /dev/null --request PUT --data-binary \
    "<CreateBucketConfiguration><LocationConstraint>$s3_region</LocationConstraint></CreateBucketConfiguration>" \
    "$SKALD_TEST_S3_URL/$1"
}

# s3_keys <bucket>: prints the keys of the objects in <bucket>, one per line.
s3_keys() {
  s3_curl --fail "$SKALD_TEST_S3_URL/$1?list-type=2" | { grep -o '<Key>[^<]*</Key>' || true; } |
    sed -e 's/^<Key>//' -e 's/<\/Key>$//'
}

# s3_stop: removes the container, if it was started.
s3_stop() {
  if [[ -n ${s3:-} ]]; then
    docker rm --force --volumes "$s3" >/dev/null
  fi
}
