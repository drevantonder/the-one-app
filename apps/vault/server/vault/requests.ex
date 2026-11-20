defmodule Vault.Requests do
  @moduledoc """
  What an app or an agent asked its person for, to answer on their own device: an item with the fields it needs, for
  an app, or for agents with no app. A request is kept in their Vault's database, and moves once (`@moves`):

      waiting --allowed with an item, or new values--> allowed
      waiting --Don't Allow----------------------------> declined
      waiting --a week with no answer (the timer)------> expired

  Each move is written only from `waiting`, so a request ends once. `expire/2`, called by the Vault's timer when the
  next request waits too long or ended a month ago (`next/2`), writes `expired` and lets go of what ended then.
  """

  alias Vault.{Access, Items, Store}

  @moves %{"waiting" => ~w(allowed declined expired)}

  # How long a request waits for its person, and how long one that ended is kept, in seconds.
  @waits 7 * 86_400
  @kept 30 * 86_400

  @doc "The states a request is in."
  def states, do: ["waiting" | @moves["waiting"]]

  @doc "Asks for an item, for `app` (an artifact) or, with none, for agents. Gives its id."
  def create(call, db, title, fields, hosts, app, by, why) do
    if fields == %{}, do: raise("A request names the fields it needs.")
    fields = for {name, about} <- fields, do: %{name: Items.field!(name), secret: Map.get(about, "secret", true)}
    id = Store.random(9)

    Store.transaction(call, db, fn d ->
      Store.query(
        d,
        "insert into requests (id, title, fields, hosts, app, by, why, state, at) values (?, ?, ?, ?, ?, ?, ?, 'waiting', ?)",
        [id, title, JSON.encode!(fields), JSON.encode!(Items.hosts!(hosts)), app, by, why, Store.now()]
      )
    end)

    id
  end

  @doc "The requests kept, newest last: those waiting, and those that ended in the last month, each with its state."
  def all(call, db, names) do
    Store.transaction(call, db, fn d ->
      for request <- Store.query(d, "select id, title, fields, hosts, app, by, why, state from requests order by at") do
        %{
          id: request["id"],
          title: request["title"],
          state: request["state"],
          fields: JSON.decode!(request["fields"]),
          hosts: JSON.decode!(request["hosts"]),
          app: request["app"] && %{name: Items.named(names, request["app"]), title: Items.title(call, request["app"])},
          by: request["by"],
          why: request["why"]
        }
      end
    end)
  end

  @doc """
  Answers a request as its person: with one of their items (`item`), with new values for a new item (`values`), or
  declining it. Allowing gives whoever asked access to the item, and lets its secrets go to the request's websites too.
  """
  def answer(call, db, id, inputs) do
    request =
      case Store.transaction(call, db, &Store.query(&1, "select * from requests where id = ? and state = 'waiting'", [id])) do
        [request] -> request
        [] -> raise "That request isn't waiting any more."
      end

    if inputs["decline"] == true do
      move!(call, db, id, "declined")
    else
      item =
        cond do
          name = inputs["item"] -> Items.find!(call, db, name)
          is_map(inputs["values"]) -> new(call, db, request, inputs["values"])
          true -> raise "Answer with one of your items, with values, or by declining."
        end

      allow!(call, db, request, item)
      move!(call, db, id, "allowed")
    end
  end

  # New values make a new item, named as the request is, or beside one of that name: never into one that's there.
  defp new(call, db, request, values) do
    wanted = JSON.decode!(request["fields"])
    missing = for %{"name" => name} <- wanted, values[name] in [nil, ""], do: name
    if missing != [], do: raise("It needs #{Enum.join(missing, ", ")} too.")
    name = free(call, db, request["title"], 1)

    Items.write!(call, db, name, Items.password(JSON.decode!(request["hosts"])), :person, fn record ->
      Enum.reduce(wanted, record, &Items.put_field(&2, &1["name"], values[&1["name"]], &1["secret"]))
    end)
  end

  defp free(call, db, title, n) do
    name = if n == 1, do: title, else: "#{title} #{n}"
    if Store.taken?(call, db, name), do: free(call, db, title, n + 1), else: name
  end

  # Allowed with an item: access for whichever app asked, or for agents if no app did, and its websites too.
  defp allow!(call, db, request, item) do
    item = Store.change!(db, item, &Items.add_hosts(&1, JSON.decode!(request["hosts"])))
    Access.switch(call, db, item, request["app"] || "agents", true)
  end

  # Moves a request from waiting, once (`@moves`).
  defp move!(call, db, id, to) do
    if to not in @moves["waiting"], do: raise("A request doesn't go from waiting to #{to}.")

    case Store.transaction(call, db, &Store.query(&1, "update requests set state = ? where id = ? and state = 'waiting' returning id", [to, id])) do
      [_moved] -> %{state: to}
      [] -> raise "That request isn't waiting any more."
    end
  end

  @doc """
  When a request next waits a week, or ended a month ago, in seconds since 1970, which the Vault's timer is set for,
  or `nil` with none.
  """
  def next(call, db) do
    Store.transaction(call, db, fn d ->
      waits = "select min(case when state = 'waiting' then at + ? else at + ? end) as next from requests"
      with [%{"next" => next}] <- Store.query(d, waits, [@waits, @kept]), do: next
    end)
  end

  @doc "What waited a week is `expired`, and what ended a month ago is let go."
  def expire(call, db) do
    now = Store.now()

    Store.transaction(call, db, fn d ->
      expired = Store.query(d, "update requests set state = 'expired' where state = 'waiting' and at < ? returning id", [now - @waits])
      Store.query(d, "delete from requests where state != 'waiting' and at < ?", [now - @kept])
      length(expired)
    end)
  end
end
