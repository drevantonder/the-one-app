import { ListCollection } from "@zag-js/collection"

/// `createListCollection({ items: [{ value: "meals", label: "Meals" }, ...] })`: what a select chooses from, as Ark
/// and Zag hold it. An item `disabled: true` can't be chosen.
export const createListCollection = (options) => new ListCollection(options)
