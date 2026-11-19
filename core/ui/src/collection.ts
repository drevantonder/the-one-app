import { ListCollection, type CollectionOptions, type CollectionItem } from "@zag-js/collection"

/// `createListCollection({ items: [{ value: "meals", label: "Meals" }, ...] })`: what a select chooses from (Zag's list
/// collection, a data structure with no DOM). An item `disabled: true` can't be chosen.
export const createListCollection = <T extends CollectionItem>(options: CollectionOptions<T>): ListCollection<T> => new ListCollection(options)
