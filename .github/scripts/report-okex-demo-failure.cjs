const title = "OKX demo contract needs attention";
const label = "okex-demo-alert";

module.exports = async function reportFailure({ github, context, alertOwner, attempt, dryRun = false }) {
  alertOwner = (alertOwner || "").trim();
  if (!/^[a-z\d](?:[a-z\d-]{0,37}[a-z\d])?$/i.test(alertOwner)) {
    throw new Error("Set OKEX_DEMO_OWNER to a repository collaborator's GitHub username.");
  }
  const { owner, repo } = context.repo;
  // Read this attempt's completed job, rather than outputs from a failed job.
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRunAttempt, {
    owner, repo, run_id: context.runId, attempt_number: attempt, per_page: 100,
  });
  const demo = jobs.find(job => job.name === "OKX demo contract");
  if (!demo || demo.status !== "completed") {
    throw new Error("Completed OKX demo contract job not found for this run attempt.");
  }
  const preflight = demo.steps.find(step =>
    step.name === "Check demo-account prerequisites" && step.conclusion !== "skipped"
  )?.conclusion;
  const category = preflight === "success"
    ? "Contract tests failed after account preflight passed."
    : preflight === "failure"
      ? "Account preflight failed. Check demo credentials, account mode, balances, and API availability before diagnosing a contract change."
      : "Workflow setup failed before account preflight completed.";
  const runUrl = `${context.serverUrl}/${owner}/${repo}/actions/runs/${context.runId}/attempts/${attempt}`;
  const body = `${category}\n\nRun: ${runUrl}\n\nOwner: @${alertOwner}. Inspect the linked logs; no credentials or account data are copied into this issue.`;
  // Smoke runs exercise failed-job classification without exchange calls or issue writes.
  if (dryRun) return body;

  await github.rest.issues.checkUserCanBeAssigned({ owner, repo, assignee: alertOwner });
  try {
    await github.rest.issues.getLabel({ owner, repo, name: label });
  } catch (error) {
    if (error.status !== 404) throw error;
    await github.rest.issues.createLabel({ owner, repo, name: label, color: "D93F0B", description: "OKX demo contract failures" });
  }
  let existing;
  for await (const { data } of github.paginate.iterator(github.rest.issues.listForRepo, {
    owner, repo, state: "open", labels: label, per_page: 100,
  })) {
    existing = data.find(issue => !issue.pull_request);
    if (existing) break;
  }
  // Reassign existing alerts too, so changing the variable rotates ownership.
  const { data: issue } = existing
    ? await github.rest.issues.addAssignees({ owner, repo, issue_number: existing.number, assignees: [alertOwner] })
    : await github.rest.issues.create({ owner, repo, title, body, labels: [label], assignees: [alertOwner] });
  if (!issue.assignees.some(user => user.login.toLowerCase() === alertOwner.toLowerCase())) {
    throw new Error(`GitHub did not assign alert #${issue.number} to OKEX_DEMO_OWNER=${alertOwner}.`);
  }
  if (existing) {
    await github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  }
  return body;
};
