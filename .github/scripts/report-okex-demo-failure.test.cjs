const test = require("node:test");
const assert = require("node:assert/strict");
const reportFailure = require("./report-okex-demo-failure.cjs");

for (const [preflight, message] of [["success", "Contract tests failed"], ["failure", "Account preflight failed"], ["skipped", "Workflow setup failed"]]) {
  test(`creates an assigned alert for ${preflight}`, async () => {
    const calls = [];
    const github = {
      paginate: async (_, args) => { assert.equal(args.state, "open"); return [{ title: "OKX demo contract needs attention", pull_request: {}, number: 1 }]; },
      rest: { issues: { listForRepo() {}, create: async args => calls.push(args) } },
    };
    await reportFailure({ github, context: { repo: { owner: "test", repo: "repo" }, serverUrl: "https://github.com", runId: 42 }, preflight });
    assert.equal(calls.length, 1);
    assert.deepEqual(calls[0].assignees, ["openoms"]);
    assert.ok(calls[0].body.includes(message));
    assert.ok(calls[0].body.includes("https://github.com/test/repo/actions/runs/42"));
  });
}

test("updates an existing alert instead of opening duplicates", async () => {
  const calls = [];
  const github = {
    paginate: async () => [{ title: "OKX demo contract needs attention", number: 7 }],
    rest: { issues: { listForRepo() {}, createComment: async args => calls.push(args) } },
  };
  await reportFailure({ github, context: { repo: { owner: "test", repo: "repo" }, serverUrl: "https://github.com", runId: 43 }, preflight: "failure" });
  assert.equal(calls.length, 1);
  assert.equal(calls[0].issue_number, 7);
});

test("reporting errors fail the alert job instead of hiding the lost notification", async () => {
  const github = { paginate: async () => { throw new Error("API unavailable"); }, rest: { issues: { listForRepo() {} } } };
  await assert.rejects(reportFailure({ github, context: { repo: { owner: "test", repo: "repo" }, serverUrl: "https://github.com", runId: 44 }, preflight: "failure" }), /API unavailable/);
});
