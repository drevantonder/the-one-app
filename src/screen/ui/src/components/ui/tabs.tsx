import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"
import { Tabs as TabsPrimitive } from "radix-ui"

function Tabs({
  className,
  orientation = "horizontal",
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Root>) {
  return (
    <TabsPrimitive.Root
      data-slot="tabs"
      data-orientation={orientation}
      orientation={orientation}
      className={cn(
        "group/tabs flex gap-2 data-[orientation=horizontal]:flex-col",
        className
      )}
      {...props}
    />
  )
}

const tabsListVariants = cva(
  "group/tabs-list inline-flex w-fit items-center justify-center rounded-lg p-[3px] text-muted-foreground group-data-[orientation=horizontal]/tabs:h-9 group-data-[orientation=vertical]/tabs:h-fit group-data-[orientation=vertical]/tabs:flex-col data-[variant=line]:rounded-none",
  {
    variants: {
      variant: {
        default: "bg-muted",
        line: "gap-1 bg-transparent",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  }
)

function TabsList({
  className,
  variant = "default",
  children,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.List> &
  VariantProps<typeof tabsListVariants>) {
  const list = React.useRef<HTMLDivElement>(null)
  const mark = React.useRef<HTMLSpanElement>(null)
  // One highlight, moved to whichever tab is on, so it slides there. It starts where it belongs, with no slide in.
  React.useLayoutEffect(() => {
    const el = list.current!
    const it = mark.current!
    const place = () => {
      const on = el.querySelector<HTMLElement>("[data-slot=tabs-trigger][data-state=active]")
      it.style.display = on ? "" : "none"
      if (!on) return
      it.style.transform = `translate(${on.offsetLeft}px, ${on.offsetTop}px)`
      it.style.width = `${on.offsetWidth}px`
      it.style.height = `${on.offsetHeight}px`
    }
    it.style.transition = "none"
    place()
    requestAnimationFrame(() => (it.style.transition = ""))
    const changed = new MutationObserver(place)
    changed.observe(el, { subtree: true, childList: true, attributes: true, attributeFilter: ["data-state"] })
    const resized = new ResizeObserver(place)
    resized.observe(el)
    return () => (changed.disconnect(), resized.disconnect())
  }, [])
  return (
    <TabsPrimitive.List
      ref={list}
      data-slot="tabs-list"
      data-variant={variant}
      className={cn(tabsListVariants({ variant }), "relative", className)}
      {...props}
    >
      <span
        ref={mark}
        aria-hidden
        data-slot="tabs-highlight"
        className={cn(
          "absolute top-0 left-0 rounded-md transition-[transform,width,height] duration-200 ease-[cubic-bezier(0.22,1,0.36,1)]",
          variant === "line"
            ? "after:absolute after:inset-x-0 after:bottom-[-5px] after:h-0.5 after:bg-foreground"
            : "border border-transparent bg-background shadow-sm dark:border-input dark:bg-input/30"
        )}
      />
      {children}
    </TabsPrimitive.List>
  )
}

function TabsTrigger({
  className,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Trigger>) {
  return (
    <TabsPrimitive.Trigger
      data-slot="tabs-trigger"
      className={cn(
        "relative inline-flex h-[calc(100%-1px)] flex-1 items-center justify-center gap-1.5 rounded-md border border-transparent px-2 py-1 text-sm font-medium whitespace-nowrap text-foreground/60 transition-colors group-data-[orientation=vertical]/tabs:w-full group-data-[orientation=vertical]/tabs:justify-start hover:text-foreground focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-1 focus-visible:outline-ring disabled:pointer-events-none disabled:opacity-50 dark:text-muted-foreground dark:hover:text-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        "data-[state=active]:text-foreground dark:data-[state=active]:text-foreground",
        className
      )}
      {...props}
    />
  )
}

function TabsContent({
  className,
  ...props
}: React.ComponentProps<typeof TabsPrimitive.Content>) {
  return (
    <TabsPrimitive.Content
      data-slot="tabs-content"
      className={cn("flex-1 outline-none", className)}
      {...props}
    />
  )
}

export { Tabs, TabsList, TabsTrigger, TabsContent, tabsListVariants }
