// What an app's page may use of `@solidjs/web` (`tsconfig.app.json` names it): a component chosen as it runs, which
// the universal renderer has (drawing-primitives/renderer.native.ts). Nothing else of it, which is the DOM's.
import type { Component, ComponentProps, Element } from "solid-js"

export declare function dynamic<T extends Component<any>>(source: () => T | undefined): Component<ComponentProps<T>>
export declare function Dynamic<T extends Component<any>>(props: { component: T | undefined } & ComponentProps<T>): Element
