// A stand-in for pi-mcp-adapter (the directory's name is what the genie guard goes by) in
// genie's tests: its `mcp` proxy tool reports the call it got, so a test sees whether the
// guard let it through. pi 1.0 cannot run the real adapter next to its native MCP support.
export default function fakeAdapter(pi: any) {
  pi.registerTool({
    name: "mcp",
    label: "MCP",
    description: "MCP proxy (test stand-in)",
    parameters: { type: "object", properties: { tool: { type: "string" }, server: { type: "string" } } },
    async execute(_id: string, params: unknown) {
      return { content: [{ type: "text", text: `FAKE-ADAPTER ${JSON.stringify(params)}` }], details: {} };
    },
  });
}
