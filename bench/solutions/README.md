# The solved fixture

A hand-written solution of every reference task, used by `npm test` to prove the
other direction of the checks: `bench/checks/r1…r8.mjs` must pass on a tree where
the acceptance criteria are met, not only fail on the shipped one:

```
copy bench/training to a temp directory, overlay these files, then
  node --test                    → green
  node bench/checks/rN.mjs <dir> → green for every N
```

`test/bench.test.ts` ("every check passes on a copy where the tasks are solved")
does exactly that, so a check that can never pass is caught by CI instead of
rotting silently.

It is **not** a reference implementation for the agents: a round must not see
these files, and `prepare` copies only `bench/training`. Keep this tree in step
with `bench/reference/tasks.json` and with the fixture: if a task changes, its
check and its solution change together.

The files mirror the fixture's layout, so copying this directory over a copy of
`bench/training` (not over the fixture itself) applies the solution:
`src/*.js` and the new test in `test/store.test.js`.
