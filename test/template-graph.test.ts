import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { layoutTemplate, templateColumns } from "../web/src/entities/agent-config/template-graph.ts";
import { NODE_H, type Relation, type TeamDef } from "../web/src/entities/agent-config/model.ts";
const teams: TeamDef[] = JSON.parse(readFileSync(new URL("../scripts/ui-preview/fixtures.json", import.meta.url), "utf8"))["/api/agent-config"].teams;
const span = (a: number, b: number, c: number, d: number) => Math.min(Math.max(a, b), Math.max(c, d)) - Math.max(Math.min(a, b), Math.min(c, d));

function verify(members: string[], relations: Relation[], columns: number) {
  const graph = layoutTemplate(members.map((key) => ({ key, label: key, sub: "" })), relations, [], columns);
  const paths = graph.edges.map((edge) => edge.points.slice(1).map((b, i) => ({ a: edge.points[i], b })));
  for (let i = 0; i < graph.edges.length; i++) {
    const edge = graph.edges[i];
    for (const point of edge.points) assert.ok(Number.isFinite(point.x) && Number.isFinite(point.y) && point.x >= 0 && point.x <= graph.width && point.y >= 0 && point.y <= graph.height);
    for (const { a, b } of paths[i]) {
      assert.ok(a.x === b.x || a.y === b.y, "routes remain orthogonal before corner rounding");
      for (const node of graph.nodes) {
        const left = node.x - graph.nodeW / 2, right = node.x + graph.nodeW / 2, top = node.y - NODE_H / 2, bottom = node.y + NODE_H / 2;
        const inside = a.y === b.y ? a.y > top && a.y < bottom && span(a.x, b.x, left, right) > 0.01 : a.x > left && a.x < right && span(a.y, b.y, top, bottom) > 0.01;
        assert.ok(!inside, `${edge.from}->${edge.to} must not pass through ${node.key}`);
      }
      for (let j = 0; j < i; j++) for (const other of paths[j]) {
        const shared = a.y === b.y && other.a.y === other.b.y && a.y === other.a.y ? span(a.x, b.x, other.a.x, other.b.x) : a.x === b.x && other.a.x === other.b.x && a.x === other.a.x ? span(a.y, b.y, other.a.y, other.b.y) : 0;
        assert.ok(shared < 0.01, `${edge.from}->${edge.to} must not share a line with ${graph.edges[j].from}->${graph.edges[j].to}`);
      }
    }
  }
  return graph;
}

for (const columns of [1, 2, 3]) test(`template arrows avoid cards and shared runs at ${columns} columns`, () => {
  for (const team of teams) {
    const graph = verify(team.members.map((m) => m.key), team.relations, columns);
    assert.equal(graph.edges.length, team.relations.reduce((count, r) => count + r.to.length, 0));
  }
});

test("opposite directions and parallel branches use different ports, including a returning cycle", () => {
  const relations: Relation[] = [
    { from: "plan", to: ["build"], type: "handoff" },
    { from: "build", to: ["test-a", "test-b", "review"], type: "handoff", on: "review" },
    { from: "build", to: ["plan"], type: "consults" },
    { from: "test-a", to: ["build"], type: "returns" },
    { from: "test-b", to: ["build"], type: "returns" },
    { from: "review", to: ["build"], type: "returns", on: "changes_requested" },
    { from: "review", to: ["orchestrator"], type: "reports" },
  ];
  for (const columns of [1, 2, 3]) verify(["plan", "build", "test-a", "test-b", "review"], relations, columns);
  verify(["a", "b"], [{ from: "a", to: ["b"], type: "handoff" }, { from: "b", to: ["a"], type: "handoff" }], 2);
});

test("card spacing leaves channels for independent arrows without shrinking labels", () => {
  const team = teams.find((t) => t.id === "full")!;
  const graph = verify(team.members.map((m) => m.key), team.relations, 3);
  assert.equal(graph.nodeW, 200);
  const a = graph.nodes[0], b = graph.nodes[1], next = graph.nodes[3];
  assert.ok(b.x - a.x - graph.nodeW >= 80);
  assert.ok(next.y - a.y - NODE_H >= 160);
  assert.equal(templateColumns(806), 3);
  assert.equal(templateColumns(326), 1);
});
