// What a strict app may use of `@solidjs/web` (`tsconfig.strict.json` names it): a component chosen as it runs, which
// both the DOM's and a native host's have. Nothing else of it, which is the DOM's.
import type { Component, ComponentProps, Element } from "solid-js"

export declare function dynamic<T extends Component<any>>(source: () => T | undefined): Component<ComponentProps<T>>
export declare function Dynamic<T extends Component<any>>(props: { component: T | undefined } & ComponentProps<T>): Element
