const title = "OKX demo contract needs attention";

module.exports = async function reportFailure({ github, context, preflight }) {
  const { owner, repo } = context.repo;
  const runUrl = `${context.serverUrl}/${owner}/${repo}/actions/runs/${context.runId}`;
  const category = preflight === "success"
    ? "Contract tests failed after account preflight passed."
    : preflight === "failure"
      ? "Account preflight failed. Check demo credentials, account mode, balances, and API availability before diagnosing a contract change."
      : "Workflow setup failed before account preflight completed.";
  const body = `${category}\n\nRun: ${runUrl}\n\nOwner: @openoms. Inspect the linked logs; no credentials or account data are copied into this issue.`;
  const issues = await github.paginate(github.rest.issues.listForRepo, { owner, repo, state: "open", per_page: 100 });
  const existing = issues.find(issue => !issue.pull_request && issue.title === title);
  if (existing) {
    await github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  } else {
    await github.rest.issues.create({ owner, repo, title, body, assignees: ["openoms"] });
  }
};
