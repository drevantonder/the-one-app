// Accounts, on a page of its own, for links that open it, as a request's own link does. Settings draws the same
// component.
import { Errored, Loading } from "solid-js"
import { Problem, render, Skeleton } from "ui"
import Accounts from "./accounts.tsx"

render(() => (
  <Errored fallback={(error) => <Problem heading="Can't show your accounts" text={error() as Error} />}>
    <Loading fallback={<Skeleton rows={4} />}>
      <Accounts />
    </Loading>
  </Errored>
))
