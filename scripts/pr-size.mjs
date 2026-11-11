// Flags PRs whose changed source lines exceed the size guide, so one task stays one small PR.
// Counts source lines only: tests, lockfiles, migrations, snapshots, docs and generated files are skipped.
// Guide: small task ~300 source lines, medium ~600, limit 800. Exits 1 above the limit unless
// SIZE_EXCEPTION=true (the PR has the `size-exception` label).
// Usage: node scripts/pr-size.mjs origin/main [limit]
import { execSync } from 'node:child_process';

const base = process.argv[2] ?? 'origin/main';
const limit = Number(process.argv[3] ?? 800);
const ignored = [
  /(^|\/)Cargo\.lock$/,
  /(^|\/)pnpm-lock\.yaml$/,
  /\.snap$/,
  /(^|\/)docs\//,
  /\.md$/,
  /\.generated\./,
  /\.(test|spec)\.[cm]?[jt]sx?$/,
  /(^|\/)(tests|e2e|__tests__)\//,
  /(^|\/)migrations\//,
];

const rows = execSync(`git diff --numstat ${base}...HEAD`, { encoding: 'utf8' }).trim().split('\n').filter(Boolean);
let total = 0;
for (const row of rows) {
  const [added, removed, file] = row.split('\t');
  if (added === '-' || ignored.some((re) => re.test(file))) continue;
  total += Number(added) + Number(removed);
}
console.log(`Changed source lines (counted): ${total} of ${limit}`);
if (total > limit) {
  if (process.env.SIZE_EXCEPTION === 'true') {
    console.log('Over the guide, but the size-exception label is set.');
  } else {
    console.log(`Over ${limit} source lines. Split the task, or add the size-exception label with a reason in the PR body.`);
    process.exit(1);
  }
}
