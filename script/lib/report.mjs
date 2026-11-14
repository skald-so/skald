// The failures of a test script in Node.js, for the release tests.
// reporter <name>: fail(<what>) prints `<name>: <what>` and counts it; failed() is the count; done()
// prints it and exits 1 when there is one, and otherwise prints that all cases passed.
export function reporter(name) {
  let failures = 0;
  return {
    fail: (what) => {
      console.error(`${name}: ${what}`);
      failures++;
    },
    failed: () => failures,
    done: () => {
      if (failures > 0) {
        console.error(`${name}: ${failures} case(s) failed`);
        process.exit(1);
      }
      console.log(`${name}: all cases passed`);
    },
  };
}
