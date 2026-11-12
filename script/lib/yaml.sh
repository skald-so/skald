# to_json <file>: a YAML file, such as a workflow, as JSON. A pinned version: the YAML 1.2 parser
# keeps the `on` key a string. Sourced by script/edge-workflow-test and script/staging-status-test.
to_json() {
  npx --yes --package yaml@2.9.1 -- yaml --json --single <"$1"
}
