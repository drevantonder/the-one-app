defmodule Claims do
  @moduledoc """
  A workplace's expense claims, kept in the plugin's database, with each receipt a blob beside it.

  A workplace's claims share one database, the plugin's own, and every row says whose it is: whoever the call that
  wrote it ran for. Who may approve is a relation tuple: an `editor` of that artifact.
  """
  @behaviour OneCore.Plugin

  @categories ~w(travel meals accommodation supplies phone other)
  # A workplace claims in one currency. Which is the network's to say, once a network has settings.
  @currency "AUD"

  @schema [
    """
    create table claims (
      id integer primary key, who text not null, title text not null, status text not null, reason text,
      sent_at text, decided_at text, paid_at text
    )
    """,
    """
    create table expenses (
      id integer primary key, who text not null, claim integer references claims (id), amount integer not null,
      date text not null, category text not null, note text not null
    )
    """,
    "create index expenses_by_claim on expenses (who, claim)",
    """
    create table receipts (
      id integer primary key, expense integer not null references expenses (id), blob text not null,
      name text not null, type text not null, size integer not null
    )
    """,
    "create index receipts_by_expense on receipts (expense)"
  ]

  @impl true
  def uses, do: ["database"]

  @impl true
  def apps, do: [%{name: "Claims", page: "app.html", icon: "icon.svg"}]

  @impl true
  def guide do
    """
    Claims is a workplace's expense claims. Someone records each expense as it happens (`record`, then `attach`
    for each receipt, whose file is an upload), gathers expenses into a claim (`gather`) and sends it (`send`). An approver lists what's
    waiting (`waiting`), opens one (`show`), looks at each receipt (`receipt`), and approves it (`approve`) or sends
    it back with the reason (`send_back`). Once the money has been paid, `pay` says so.

    - A claim is `draft`, `sent`, `sent_back`, `approved` or `paid`. Its expenses change only while it's `draft` or
      `sent_back`: put them right, then `send` it again.
    - An amount is a number in #{@currency}: 12.5 is $12.50. A date is written 2026-11-14.
    - Sending a claim, approving one, sending one back and saying one is paid are a person's decisions. Do them
      when asked to, and say what you did.
    - An approver is an editor of Claims' own artifact, which whoever owns it says with `relations share`.
      Being refused names the artifact. Everyone else sees only their own expenses and claims.
    """
  end

  @impl true
  def tools do
    expense = %{type: "integer", description: "The expense's id"}
    claim = %{type: "integer", description: "The claim's id"}
    receipt = %{type: "integer", description: "The receipt's id"}

    said = %{
      amount: %{type: "number", description: "How much, in #{@currency}: 12.5 is $12.50"},
      date: %{type: "string", description: "The day it was spent, as 2026-11-14. Today if left out"},
      category: %{type: "string", enum: @categories, description: "What it was for. other if left out"},
      note: %{type: "string", description: "What it was, in a few words"}
    }

    of = fn inputs, required -> %{type: "object", properties: inputs, required: required} end

    [
      %{name: "record", description: "Records an expense", inputs: of.(said, ["amount"])},
      %{
        name: "change",
        description: "Changes an expense, unless its claim is sent, approved or paid",
        inputs: of.(Map.put(said, :expense, expense), ["expense"])
      },
      %{
        name: "remove",
        description: "Removes an expense and its receipts, unless its claim is sent, approved or paid",
        inputs: of.(%{expense: expense}, ["expense"])
      },
      %{
        name: "attach",
        description: "Adds a receipt to an expense: a photo, or any other file",
        inputs:
          of.(
            %{
              expense: expense,
              name: %{type: "string", description: "The file's name, such as taxi.jpg"},
              type: %{type: "string", description: "What kind of file it is, such as image/jpeg"},
              upload: %{type: "string", format: "upload", description: "The file, as the name `upload` gave it"}
            },
            ["expense", "name", "type", "upload"]
          )
      },
      %{
        name: "detach",
        description: "Removes a receipt from its expense, unless its claim is sent, approved or paid",
        inputs: of.(%{receipt: receipt}, ["receipt"])
      },
      %{
        name: "receipt",
        description: "Gives a receipt's file: its photo, to look at",
        read_only: true,
        fixed: true,
        inputs: of.(%{receipt: receipt}, ["receipt"])
      },
      %{
        name: "expenses",
        description: "Lists your expenses that aren't in a claim yet, newest first",
        read_only: true,
        inputs: of.(%{}, [])
      },
      %{
        name: "gather",
        description: "Gathers expenses into a new claim, or into one that's a draft or was sent back",
        inputs:
          of.(
            %{
              expenses: %{type: "array", items: %{type: "integer"}, description: "The expenses' ids, as [1, 2]"},
              title: %{type: "string", description: "The new claim's name, such as Sydney trip"},
              claim: %{type: "integer", description: "The claim to add them to, instead of making one"}
            },
            ["expenses"]
          )
      },
      %{
        name: "take_out",
        description: "Takes an expense out of a claim that's a draft or was sent back",
        inputs: of.(%{expense: expense}, ["expense"])
      },
      %{
        name: "discard",
        description: "Removes a claim that's a draft or was sent back. Its expenses stay, in no claim",
        inputs: of.(%{claim: claim}, ["claim"])
      },
      %{name: "send", description: "Sends a claim to be approved", inputs: of.(%{claim: claim}, ["claim"])},
      %{
        name: "list",
        description: "Lists your claims, newest first, each with its total and where it's up to",
        read_only: true,
        inputs: of.(%{}, [])
      },
      %{
        name: "show",
        description: "Gives one claim with its expenses and their receipts",
        read_only: true,
        inputs: of.(%{claim: claim}, ["claim"])
      },
      %{
        name: "waiting",
        description:
          "Lists the claims that wait on you as an approver: sent, to approve or send back, and approved, to pay",
        read_only: true,
        inputs: of.(%{}, [])
      },
      %{name: "approve", description: "Approves a claim that was sent", inputs: of.(%{claim: claim}, ["claim"])},
      %{
        name: "send_back",
        description: "Sends a claim back to whoever made it, with the reason",
        inputs:
          of.(
            %{claim: claim, reason: %{type: "string", description: "What needs to change, in a sentence"}},
            ["claim", "reason"]
          )
      },
      %{name: "pay", description: "Says an approved claim has been paid", inputs: of.(%{claim: claim}, ["claim"])}
    ]
  end

  # Whose expenses and claims a call is about: whoever it runs for.
  defp who(call), do: call.for

  # Whether a call's user may approve, send back and pay: an editor of the workplace's claims, which are this
  # plugin's own artifact. With no plugin deciding who reaches what, everyone is.
  defp approver?(call), do: OneCore.Call.reaches?(call, OneCore.Artifact.name(call.own), "editor")

  defp approver!(call) do
    if not approver?(call) do
      raise "Only an approver does that: an editor of #{OneCore.Artifact.name(call.own)}, which #{who(call)} isn't."
    end
  end

  @impl true
  def call("record", %{"amount" => amount} = inputs, call) do
    said = [
      who(call),
      cents(amount),
      date(Map.get(inputs, "date", Date.to_iso8601(Date.utc_today()))),
      category(Map.get(inputs, "category", "other")),
      Map.get(inputs, "note", "")
    ]

    transaction(call, fn db ->
      [%{"id" => id}] =
        query(db, "insert into expenses (who, amount, date, category, note) values (?, ?, ?, ?, ?) returning id", said)

      expense(db, id)
    end)
  end

  def call("change", %{"expense" => id} = inputs, call) do
    said =
      for {name, check} <- [{"amount", &cents/1}, {"date", &date/1}, {"category", &category/1}, {"note", & &1}],
          is_map_key(inputs, name),
          do: {name, check.(inputs[name])}

    transaction(call, fn db ->
      open_expense!(db, call, id)
      for {name, value} <- said, do: query(db, "update expenses set #{name} = ? where id = ?", [value, id])
      expense(db, id)
    end)
  end

  def call("remove", %{"expense" => id}, call) do
    transaction(call, fn db ->
      open_expense!(db, call, id)
      query(db, "delete from receipts where expense = ?", [id])
      query(db, "delete from expenses where id = ?", [id])
      "Removed expense #{id}."
    end)
  end

  # A receipt's file was sent straight to the bucket, into this plugin's own artifact, and stays where it landed.
  def call("attach", %{"expense" => id, "name" => name, "type" => type, "upload" => upload}, call) do
    size =
      case String.starts_with?(upload, "uploads/") and OneCore.Artifact.size(call.own, upload) do
        {:ok, size} when size > 0 -> size
        _ -> raise "That file didn't arrive. Send it again."
      end

    transaction(call, fn db ->
      open_expense!(db, call, id)

      [receipt] =
        query(
          db,
          "insert into receipts (expense, blob, name, type, size) values (?, ?, ?, ?, ?) returning id, name, type, size",
          [id, upload, name, type, size]
        )

      receipt
    end)
  end

  def call("detach", %{"receipt" => id}, call) do
    transaction(call, fn db ->
      case query(db, "select expense from receipts where id = ?", [id]) do
        [%{"expense" => expense}] -> open_expense!(db, call, expense)
        [] -> raise "There's no receipt #{id}."
      end

      query(db, "delete from receipts where id = ?", [id])
      "Removed receipt #{id}."
    end)
  end

  def call("receipt", %{"receipt" => id}, call) do
    mine = "select blob, type from receipts where id = ? and expense in (select id from expenses where who = ? or ?)"

    case transaction(call, &query(&1, mine, [id, who(call), approver?(call)])) do
      [%{"blob" => blob, "type" => type}] ->
        {:file, type, call.own, blob}

      [] ->
        raise "There's no receipt #{id}."
    end
  end

  def call("expenses", _inputs, call) do
    transaction(call, &expenses(&1, "who = ? and claim is null", [who(call)]))
  end

  def call("gather", %{"expenses" => ids} = inputs, call) do
    transaction(call, fn db ->
      claim =
        case inputs do
          %{"claim" => claim} ->
            open_claim!(db, call, claim)
            claim

          %{"title" => title} ->
            if String.trim(title) == "", do: raise("A claim needs a name.")
            values = [who(call), String.trim(title)]

            [%{"id" => claim}] =
              query(db, "insert into claims (who, title, status) values (?, ?, 'draft') returning id", values)

            claim

          _ ->
            raise "A new claim needs a title, and an existing one its id as claim."
        end

      for id <- ids do
        gathered = "update expenses set claim = ? where id = ? and who = ? and claim is null returning id"

        if query(db, gathered, [claim, id, who(call)]) == [],
          do: raise("Expense #{id} isn't there, or is in a claim already.")
      end

      claim(db, claim)
    end)
  end

  def call("take_out", %{"expense" => id}, call) do
    transaction(call, fn db ->
      open_expense!(db, call, id)
      query(db, "update expenses set claim = null where id = ?", [id])
      expense(db, id)
    end)
  end

  def call("discard", %{"claim" => id}, call) do
    transaction(call, fn db ->
      open_claim!(db, call, id)
      query(db, "update expenses set claim = null where claim = ?", [id])
      query(db, "delete from claims where id = ?", [id])
      "Removed claim #{id}. Its expenses are in no claim."
    end)
  end

  def call("send", %{"claim" => id}, call) do
    transaction(call, fn db ->
      open_claim!(db, call, id)

      if query(db, "select 1 from expenses where claim = ? limit 1", [id]) == [],
        do: raise("A claim needs an expense before it's sent.")

      query(db, "update claims set status = 'sent', reason = null, decided_at = null, sent_at = ? where id = ?", [
        now(),
        id
      ])

      claim(db, id)
    end)
  end

  def call("list", _inputs, call) do
    transaction(call, &claims(&1, "who = ?", [who(call)]))
  end

  # A claim is shown to whoever made it and to an approver.
  def call("show", %{"claim" => id}, call) do
    whose = if approver?(call), do: [], else: [who(call)]
    transaction(call, &claim(&1, id, whose))
  end

  # Nothing waits on someone who isn't an approver.
  def call("waiting", _inputs, call) do
    if approver?(call), do: transaction(call, &claims(&1, "status in ('sent', 'approved')", [])), else: []
  end

  def call("approve", %{"claim" => id}, call) do
    decide(call, id, "sent", "status = 'approved', decided_at = ?", [now()])
  end

  def call("send_back", %{"claim" => id, "reason" => reason}, call) do
    if String.trim(reason) == "", do: raise("Say what needs to change.")
    decide(call, id, "sent", "status = 'sent_back', reason = ?, decided_at = ?", [String.trim(reason), now()])
  end

  def call("pay", %{"claim" => id}, call) do
    decide(call, id, "approved", "status = 'paid', paid_at = ?", [now()])
  end

  # An approver moves a claim on from the one status it can be decided in.
  defp decide(call, id, from, set, values) do
    approver!(call)

    transaction(call, fn db ->
      case query(db, "update claims set #{set} where id = ? and status = ? returning id", values ++ [id, from]) do
        [_] -> claim(db, id)
        [] -> raise "Claim #{id} isn't #{from}, or isn't there."
      end
    end)
  end

  # The database module goes with the database it opened, so one `query` serves every tool.
  defp transaction(call, fun) do
    database = OneCore.Call.plugin(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values), do: database.query(db, sql, values)

  defp now, do: DateTime.to_iso8601(DateTime.utc_now(:second))

  defp cents(amount) when is_number(amount) and amount > 0, do: round(amount * 100)
  defp cents(_amount), do: raise("An amount is a number above 0.")

  defp date(date) do
    case is_binary(date) and Date.from_iso8601(date) do
      {:ok, _} -> date
      _ -> raise "A date is written 2026-11-14."
    end
  end

  defp category(category) when category in @categories, do: category
  defp category(_category), do: raise("A category is one of #{Enum.join(@categories, ", ")}.")

  # Raises unless a claim is the caller's and can still change.
  defp open_claim!(db, call, id) do
    case query(db, "select status from claims where id = ? and who = ?", [id, who(call)]) do
      [%{"status" => status}] when status in ["draft", "sent_back"] -> :ok
      [%{"status" => status}] -> raise "Claim #{id} is #{status}, so it can't change."
      [] -> raise "There's no claim #{id}."
    end
  end

  # Raises unless an expense is the caller's and can still change.
  defp open_expense!(db, call, id) do
    case query(db, "select claim from expenses where id = ? and who = ?", [id, who(call)]) do
      [%{"claim" => nil}] -> :ok
      [%{"claim" => claim}] -> open_claim!(db, call, claim)
      [] -> raise "There's no expense #{id}."
    end
  end

  defp expense(db, id), do: hd(expenses(db, "id = ?", [id]))

  # Expenses as they're answered: the amount as a number of dollars, and each one's receipts with it.
  defp expenses(db, which, values) do
    receipts =
      db
      |> query(
        "select id, expense, name, type, size from receipts where expense in (select id from expenses where #{which}) order by id",
        values
      )
      |> Enum.group_by(& &1["expense"], &Map.delete(&1, "expense"))

    for expense <-
          query(
            db,
            "select id, claim, amount, date, category, note from expenses where #{which} order by date desc, id desc",
            values
          ) do
      expense |> Map.update!("amount", &(&1 / 100)) |> Map.put("receipts", Map.get(receipts, expense["id"], []))
    end
  end

  # A claim with its expenses: the one with this id, or only if it's `whose`.
  defp claim(db, id, whose \\ []) do
    which = if whose == [], do: "id = ?", else: "id = ? and who = ?"

    case claims(db, which, [id | whose]) do
      [claim] -> Map.put(claim, "expenses", expenses(db, "claim = ?", [id]))
      [] -> raise "There's no claim #{id}."
    end
  end

  # Claims as they're answered, each with its total as a number of dollars.
  defp claims(db, which, values) do
    listed = """
    select id, title, status, reason, sent_at, decided_at, paid_at,
      (select coalesce(sum(amount), 0) from expenses where claim = claims.id) as total
    from claims where #{which} order by id desc
    """

    for claim <- query(db, listed, values), do: Map.update!(claim, "total", &(&1 / 100))
  end
end
