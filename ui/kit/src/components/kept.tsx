import * as React from "react"

/// An element the client made and keeps, put here as it is: its frames go on running through every draw.
function Kept({ element, className = "contents" }: { element: HTMLElement; className?: string }) {
  const ref = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    if (element.parentElement !== ref.current) ref.current?.append(element)
  }, [element])
  return <div ref={ref} className={className} />
}

export { Kept }
