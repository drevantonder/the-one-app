// A tiny app for the test of tools' answers held to their `output`: one that keeps to it, one that doesn't say, and
// three that break it, by a wrong type, a missing field and a field it doesn't list.

const none = { type: "object", properties: {} };
const count = { type: "object", required: ["n"], properties: { n: { type: "integer" } } };

export default {
  name: "answers",
  title: "Answers",
  description: "Answers, for tests",
  tools: {
    keeps: { reads: true, title: "Keeps to it", description: "n.", input: none, output: count, run: async () => ({ n: 1 }) },
    unsaid: { reads: true, description: "Says nothing of its answer.", input: none, run: async () => ({ anything: [1, "two"] }) },
    wrong_type: { reads: true, title: "Wrong type", description: "n.", input: none, output: count, run: async () => ({ n: "one" }) },
    missing: { reads: true, title: "Missing", description: "n.", input: none, output: count, run: async () => ({}) },
    extra: { reads: true, title: "Extra", description: "n.", input: none, output: count, run: async () => ({ n: 1, m: 2 }) },
  },
};
