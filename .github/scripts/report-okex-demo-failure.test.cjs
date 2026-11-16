const test = require("node:test");
const assert = require("node:assert/strict");
const reportFailure = require("./report-okex-demo-failure.cjs");
const context = { repo: { owner: "test", repo: "repo" }, serverUrl: "https://github.com", runId: 42 };

function harness({ preflight = "success", pages = [[]], jobs, missingLabel = false, droppedAssignee = false, failure } = {}) {
  const calls = [];
  let pagesRead = 0;
  const endpoint = (name, result) => async args => {
    calls.push([name, args]);
    if (failure === name) throw new Error(`${name} unavailable`);
    if (name === "getLabel" && missingLabel) throw Object.assign(new Error("Not found"), { status: 404 });
    return result;
  };
  const assigned = { data: { number: 8, assignees: droppedAssignee ? [] : [{ login: "Maintainer" }] } };
  const github = {
    rest: {
      actions: { listJobsForWorkflowRunAttempt: endpoint("listJobs") },
      issues: Object.fromEntries(["checkUserCanBeAssigned", "getLabel", "createLabel", "listForRepo", "addAssignees", "create", "createComment"].map(name => [name, endpoint(name, assigned)])),
    },
    paginate: async (method, args) => {
      assert.equal(method, github.rest.actions.listJobsForWorkflowRunAttempt);
      assert.deepEqual(args, { owner: "test", repo: "repo", run_id: 42, attempt_number: 2, per_page: 100 });
      await method(args);
      return jobs || [{ name: "OKX demo contract", status: "completed", conclusion: "failure", steps: [
        { name: "Check demo-account prerequisites", conclusion: "skipped" },
        { name: "Check demo-account prerequisites", conclusion: preflight },
      ] }];
    },
  };
  github.paginate.iterator = async function* (method, args) {
    assert.equal(method, github.rest.issues.listForRepo);
    assert.deepEqual(args, { owner: "test", repo: "repo", state: "open", labels: "okex-demo-alert", per_page: 100 });
    await method(args);
    for (const data of pages) { pagesRead++; yield { data }; }
  };
  return {
    calls, pagesRead: () => pagesRead,
    run: args => reportFailure({ github, context, alertOwner: "maintainer", attempt: 2, ...args }),
  };
}

for (const [preflight, message] of [["success", "Contract tests failed"], ["failure", "Account preflight failed"], ["skipped", "Workflow setup failed"]]) {
  test(`classifies a failed job with ${preflight} preflight and creates an assigned labeled alert`, async () => {
    const h = harness({ preflight });
    const body = await h.run();
    assert.ok(body.startsWith(message));
    const created = h.calls.find(([name]) => name === "create")[1];
    assert.deepEqual(created.assignees, ["maintainer"]);
    assert.deepEqual(created.labels, ["okex-demo-alert"]);
    assert.equal(created.body, body);
    assert.ok(body.includes("https://github.com/test/repo/actions/runs/42/attempts/2"));
    assert.ok(body.includes("Owner: @maintainer"));
    assert.ok(!h.calls.some(([name]) => name === "createLabel"));
  });
  test(`smoke test classifies ${preflight} without issue API calls`, async () => {
    const h = harness({ preflight });
    assert.ok((await h.run({ dryRun: true })).startsWith(message));
    assert.deepEqual(h.calls.map(([name]) => name), ["listJobs"]);
  });
}

test("finds a renamed labeled issue on a later page, ignores PRs and rotates ownership", async () => {
  const first = Array.from({ length: 100 }, (_, i) => ({ number: i + 1, pull_request: {}, title: "OKX demo contract needs attention" }));
  const h = harness({ pages: [first, [{ number: 107, title: "Investigating demo account" }], [{ number: 108 }]] });
  await h.run();
  assert.equal(h.pagesRead(), 2);
  assert.ok(!h.calls.some(([name]) => name === "create"));
  assert.deepEqual(h.calls.find(([name]) => name === "addAssignees")[1], { owner: "test", repo: "repo", issue_number: 107, assignees: ["maintainer"] });
  assert.equal(h.calls.find(([name]) => name === "createComment")[1].issue_number, 107);
});

test("creates the dedicated label when it does not exist", async () => {
  const h = harness({ missingLabel: true });
  await h.run();
  assert.equal(h.calls.find(([name]) => name === "createLabel")[1].name, "okex-demo-alert");
});

for (const alertOwner of [undefined, "", "  ", "@maintainer", "org/team"]) {
  test(`rejects missing or invalid owner ${JSON.stringify(alertOwner)}`, async () => {
    const h = harness();
    await assert.rejects(h.run({ alertOwner }), /Set OKEX_DEMO_OWNER/);
    assert.deepEqual(h.calls, []);
  });
}

test("trims the configured username", async () => {
  const h = harness();
  await h.run({ alertOwner: " maintainer " });
  assert.equal(h.calls.find(([name]) => name === "checkUserCanBeAssigned")[1].assignee, "maintainer");
});

for (const jobs of [[], [{ name: "Other job", status: "completed" }], [{ name: "OKX demo contract", status: "in_progress" }]]) {
  test(`rejects unavailable completed job: ${JSON.stringify(jobs)}`, async () => {
    await assert.rejects(harness({ jobs }).run(), /Completed OKX demo contract job not found/);
  });
}

test("failure before preflight appears in the job is a setup failure", async () => {
  assert.ok((await harness({ jobs: [{ name: "OKX demo contract", status: "completed", steps: [] }] }).run()).startsWith("Workflow setup failed"));
});

for (const pages of [[[]], [[{ number: 7 }]]]) {
  test(`fails loudly if GitHub silently drops the assignee (existing: ${!!pages[0].length})`, async () => {
    const h = harness({ pages, droppedAssignee: true });
    await assert.rejects(h.run(), /GitHub did not assign alert/);
    assert.ok(!h.calls.some(([name]) => name === "createComment"));
  });
}

for (const failure of ["listJobs", "checkUserCanBeAssigned", "getLabel", "createLabel", "listForRepo", "create", "addAssignees", "createComment"]) {
  test(`propagates ${failure} failures`, async () => {
    const pages = [failure === "addAssignees" || failure === "createComment" ? [{ number: 7 }] : []];
    await assert.rejects(harness({ failure, pages, missingLabel: failure === "createLabel" }).run(), new RegExp(`${failure} unavailable`));
  });
}
