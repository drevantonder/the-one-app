// A stylesheet imported as its text, as Vite gives it with `?inline`.
declare module "*.css?inline" {
  const css: string
  export default css
}
