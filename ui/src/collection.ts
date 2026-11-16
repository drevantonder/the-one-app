import { ListCollection, type CollectionOptions, type CollectionItem } from "@zag-js/collection"

/// `createListCollection({ items: [{ value: "meals", label: "Meals" }, ...] })`: what a select chooses from, as Ark
/// and Zag hold it. An item `disabled: true` can't be chosen.
export const createListCollection = <T extends CollectionItem>(options: CollectionOptions<T>): ListCollection<T> => new ListCollection(options)
