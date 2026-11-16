// A Markdown file imported is its text, as `wrangler.jsonc`'s rules have it.
declare module "*.md" {
  const text: string;
  export default text;
}
