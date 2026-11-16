defmodule Gmail do
  @moduledoc """
  Gmail: the person's own Gmail accounts, through Gmail's API (v1). Signing in and the accounts are the `google`
  plugin's, the same for every Google app. Reading a message is `Gmail.Message`'s, and writing one is
  `Gmail.Mime`'s.
  """
  @behaviour OneCore.Plugin
  alias Gmail.{Message, Mime}

  @api "https://gmail.googleapis.com/gmail/v1/users/me/"
  @upload "https://gmail.googleapis.com/upload/gmail/v1/users/me/"

  @doc "What Gmail asks of Google, for the `google` plugin."
  def google do
    %{
      title: "Gmail",
      about: "Sign in to Gmail, and your agents can search, read and draft your email.",
      scopes: ["https://www.googleapis.com/auth/gmail.modify"]
    }
  end

  @impl true
  def uses, do: ["google", "vault"]

  @impl true
  def tools do
    account = %{
      type: "string",
      description: "Which account: its address, or a part of it only it has, as gmail_accounts lists them. The first if left out"
    }

    with_account = fn properties, required ->
      Map.merge(%{type: "object", properties: Map.put(properties, :account, account)}, if(required, do: %{required: required}, else: %{}))
    end

    string = &%{type: "string", description: &1}
    ids = &%{type: "array", items: %{type: "string"}, description: &1}
    people = &%{type: "array", items: %{type: "string"}, description: "#{&1}: addresses, each a@example.org or Name <a@example.org>"}
    most = &%{type: "integer", description: "At most this many. #{&1} if left out, and at most 100"}
    page = &string.("The next page: the token the last #{&1} ended with (More: give pageToken ...), with the same other inputs")
    id = string.("The message's id, from gmail_search")

    writing = %{
      to: people.("Who it's to. A reply goes to the sender unless given"),
      cc: people.("Copied to"),
      bcc: people.("Copied to, unseen"),
      subject: string.("The subject. A reply keeps the original's, with Re:, unless given"),
      body: string.("The email's text, as plain text"),
      html: string.("The email as HTML. It's sent with body, or without it, the HTML made plain, as its text, so mail apps show the one they prefer"),
      reply_to: string.("A message's id, from gmail_search or gmail_read, to reply to it: the reply goes in its thread"),
      attachments: %{
        type: "array",
        description: "Files to attach, up to 10 and 25 MB in all",
        items: %{
          type: "object",
          required: ["name"],
          properties: %{
            name: string.("The file's name, like receipt.pdf"),
            type: string.("Its MIME type, like application/pdf. Guessed from the name if left out"),
            base64: %{type: "string", contentEncoding: "base64", description: "Its bytes, in base64"},
            upload: %{type: "string", format: "upload", description: "Or the file, uploaded"}
          }
        }
      }
    }

    [
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts Gmail can use, the default first, each with ok, and why when it's false (gmail_sign_in with that account fixes it). Every other tool's account takes one of these.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "sign_in",
        description:
          "Starts signing a Google account in to Gmail, or letting Gmail use one another Google app has: gives url, for the person to open in their browser, where Google signs them in and sends them back to finish.",
        inputs: %{type: "object", properties: %{account: string.("The account's address, like someone@gmail.com, to suggest it to Google: for one that needs signing in again, as gmail_accounts lists it")}}
      },
      %{
        name: "sign_out",
        description: "Signs a Google account out of Gmail. Other Google apps that use it still do, and gmail_sign_in signs it in again.",
        inputs: with_account.(%{}, ["account"])
      },
      %{
        name: "profile",
        read_only: true,
        description: "The account's address and how many messages and threads it has.",
        inputs: with_account.(%{}, nil)
      },
      %{
        name: "search",
        read_only: true,
        description:
          "Finds messages (Gmail's messages.list), newest first, as text: each message's id, when it came (ISO 8601, UTC), sender, subject, label names and a snippet. With no q, all mail but spam and trash. Read one with gmail_read, its conversation with gmail_thread. A long list ends with More: give pageToken <token>.",
        inputs:
          with_account.(
            %{
              q: string.("A search as in Gmail's search box, like in:inbox is:unread from:someone@example.com newer_than:7d"),
              labelIds: ids.("Only messages with all these labels: label ids, as gmail_labels lists them, like INBOX or Label_12"),
              maxResults: most.(20),
              pageToken: page.("search")
            },
            nil
          )
      },
      %{
        name: "read",
        read_only: true,
        description:
          "One message in full: its headers, when it came, labels, attachments (numbered, for gmail_attachment) and its text. gmail_thread gives the replies around it.",
        inputs: with_account.(%{id: id}, ["id"])
      },
      %{
        name: "thread",
        read_only: true,
        description: "A whole conversation, oldest first, each message as gmail_read gives it. gmail_read gives just the one.",
        inputs: with_account.(%{id: string.("The thread's id, or the id of any message in it, from gmail_search")}, ["id"])
      },
      %{
        name: "attachment",
        read_only: true,
        description:
          "One of a message's attachments, as a file: an agent sees it, and the command writes it out (one gmail attachment --id ... > receipt.pdf).",
        inputs:
          with_account.(
            %{
              id: id,
              name: string.("The attachment's file name, or a part of it, as gmail_read lists them. Or give number"),
              number: %{type: "integer", description: "Which attachment, as gmail_read numbers them, 1 the first. The first when neither is given"}
            },
            ["id"]
          )
      },
      %{
        name: "labels",
        read_only: true,
        description:
          "The account's labels, Gmail's own (INBOX, UNREAD, STARRED, ...) first, then the person's: each its name, and its id where that differs. gmail_label and gmail_search's labelIds take either.",
        inputs: with_account.(%{}, nil)
      },
      %{
        name: "label",
        description:
          "Adds labels to and removes labels from messages (Gmail's messages.batchModify). Archive: remove INBOX. Mark read: remove UNREAD. Star: add STARRED. Says what changed.",
        inputs:
          with_account.(
            %{
              ids: ids.("The messages' ids, from gmail_search"),
              addLabelIds: ids.("Labels to add: ids or names, as gmail_labels lists them, like STARRED or Receipts"),
              removeLabelIds: ids.("Labels to remove: ids or names, like INBOX or UNREAD")
            },
            ["ids"]
          )
      },
      %{
        name: "trash",
        description: "Moves messages to the trash. They stay there 30 days, where the person can take them out again in Gmail.",
        inputs: with_account.(%{ids: ids.("The messages' ids, from gmail_search")}, ["ids"])
      },
      %{
        name: "drafts",
        read_only: true,
        description:
          "The account's drafts (Gmail's drafts.list), newest first, as text: each draft's id, when it was saved (ISO 8601, UTC), who it's to and its subject. A long list ends with More: give pageToken <token>.",
        inputs: with_account.(%{maxResults: most.(20), pageToken: page.("drafts")}, nil)
      },
      %{
        name: "draft",
        description:
          "Saves an email as a draft (text, HTML or both, with files if given) for the person to check and send, or replaces a draft. Use it rather than gmail_send unless the person asked you to send. Gives the draft's id.",
        inputs: with_account.(Map.put(writing, :id, string.("A draft's id, from gmail_drafts, to replace that draft")), nil)
      },
      %{
        name: "send_draft",
        description: "Sends a draft as it is. This sends real email to real people: only when the person asked to send this one.",
        inputs: with_account.(%{id: string.("The draft's id, from gmail_drafts or gmail_draft")}, ["id"])
      },
      %{
        name: "delete_draft",
        description: "Deletes a draft for good: it doesn't go to the trash.",
        inputs: with_account.(%{id: string.("The draft's id, from gmail_drafts")}, ["id"])
      },
      %{
        name: "send",
        description:
          "Sends an email (text, HTML or both, with files if given), or a reply. This sends real email to real people: only what the person asked to send. Otherwise gmail_draft. Gives the sent message's id.",
        inputs: with_account.(writing, nil)
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) when tool in ~w(accounts sign_in sign_out),
    do: google(call).account_call(tool, inputs, call, google())

  def call(tool, inputs, call) do
    email = google(call).account(call, inputs["account"])
    run(tool, inputs, %{call: call, email: email})
  end

  defp run("profile", _inputs, at) do
    profile = api(at, "GET", "profile")
    "#{profile["emailAddress"]}: #{profile["messagesTotal"]} messages in #{profile["threadsTotal"]} threads."
  end

  defp run("search", inputs, at) do
    found =
      api(at, "GET", "messages",
        query: [
          maxResults: most(inputs["maxResults"]),
          q: inputs["q"],
          labelIds: list(inputs["labelIds"]),
          pageToken: inputs["pageToken"]
        ]
      )

    case found["messages"] do
      messages when messages in [nil, []] ->
        if inputs["pageToken"], do: "No more.", else: "Nothing found."

      messages ->
        names = label_names(at)

        lines =
          for message <- metadata(at, messages) do
            labels = Message.labels(message, names)

            Enum.join(
              [
                "#{message["id"]}  #{Message.time(message)}  #{Message.header(message["payload"], "From")}",
                "  #{Message.header(message["payload"], "Subject")}",
                labels != "" && "  [#{labels}]",
                message["snippet"] not in [nil, ""] && "  #{Message.snippet(message["snippet"])}"
              ]
              |> Enum.filter(&is_binary/1),
              "\n"
            )
          end

        Enum.join(lines, "\n") <> more(found["nextPageToken"])
    end
  end

  defp run("read", %{"id" => id}, at) do
    message = api(at, "GET", "messages/#{path(at, id)}", query: [format: "full"], missing: no_message(id))
    Message.show(message, label_names(at))
  end

  defp run("thread", %{"id" => id}, at) do
    thread =
      try do
        api(at, "GET", "threads/#{path(at, id)}", query: [format: "full"], missing: no_message(id))
      rescue
        error ->
          # Or a message's id, whose thread this is.
          message = api(at, "GET", "messages/#{path(at, id)}", query: [format: "minimal"], missing: no_message(id))
          if message["threadId"] == id, do: reraise(error, __STACKTRACE__)
          api(at, "GET", "threads/#{path(at, message["threadId"])}", query: [format: "full"])
      end

    names = label_names(at)
    Enum.map_join(thread["messages"], "\n\n----------\n\n", &Message.show(&1, names))
  end

  defp run("attachment", %{"id" => id} = inputs, at) do
    message = api(at, "GET", "messages/#{path(at, id)}", query: [format: "full"], missing: no_message(id))
    all = Message.attachments(message["payload"])
    if all == [], do: raise("Message #{id} has no attachments.")

    file =
      case inputs do
        %{"name" => name} when is_binary(name) and name != "" ->
          Enum.find(all, &(&1.name == name)) ||
            Enum.find(all, &String.contains?(String.downcase(&1.name), String.downcase(name)))

        _ ->
          Enum.at(all, (inputs["number"] || 1) - 1)
      end

    file ||
      raise "No attachment #{inputs["name"] || inputs["number"]} on message #{id}. It has: #{all |> Enum.with_index(1) |> Enum.map_join(", ", fn {file, at} -> "#{at}. #{file.name}" end)}."

    data =
      if file.id,
        do: api(at, "GET", "messages/#{path(at, message["id"])}/attachments/#{path(at, file.id)}")["data"],
        else: file.data

    {:blob, file.type, Message.bytes(data)}
  end

  defp run("labels", _inputs, at) do
    labels = api(at, "GET", "labels")["labels"] || []

    labels
    |> Enum.sort_by(&{&1["type"], String.downcase(&1["name"])})
    |> Enum.map_join("\n", fn label ->
      if label["id"] != label["name"], do: "#{label["name"]}  (id #{label["id"]})", else: label["name"]
    end)
  end

  defp run("label", inputs, at) do
    which = list(inputs["ids"])
    if which == [], do: raise("Give ids: messages' ids, from gmail_search.")

    if list(inputs["addLabelIds"]) ++ list(inputs["removeLabelIds"]) == [],
      do: raise("Give addLabelIds or removeLabelIds: label ids or names, as gmail_labels lists them.")

    labels = api(at, "GET", "labels")["labels"] || []

    resolve = fn given ->
      for name <- list(given) do
        Enum.find(labels, &(&1["id"] == name or String.downcase(&1["name"]) == String.downcase(name))) ||
          raise "No label \"#{name}\": gmail_labels lists them."
      end
    end

    {adding, removing} = {resolve.(inputs["addLabelIds"]), resolve.(inputs["removeLabelIds"])}

    api(at, "POST", "messages/batchModify",
      json: %{ids: which, addLabelIds: Enum.map(adding, & &1["id"]), removeLabelIds: Enum.map(removing, & &1["id"])},
      missing: "No message among #{Enum.join(which, ", ")}: message ids come from gmail_search."
    )

    did =
      [
        adding != [] && "added #{Enum.map_join(adding, ", ", & &1["name"])}",
        removing != [] && "removed #{Enum.map_join(removing, ", ", & &1["name"])}"
      ]
      |> Enum.filter(&is_binary/1)
      |> Enum.join(" and ")

    String.capitalize(String.first(did)) <> String.slice(did, 1..-1//1) <> " on #{messages(length(which))}."
  end

  defp run("trash", inputs, at) do
    which = list(inputs["ids"])
    if which == [], do: raise("Give ids: messages' ids, from gmail_search.")

    which
    |> Enum.with_index()
    |> Enum.each(fn {id, done} ->
      try do
        api(at, "POST", "messages/#{path(at, id)}/trash", missing: no_message(id))
      rescue
        error in RuntimeError ->
          if done == 0, do: reraise(error, __STACKTRACE__)
          raise "Trashed #{done} of #{length(which)}, then: #{error.message}"
      end
    end)

    "Moved #{messages(length(which))} to the trash."
  end

  defp run("drafts", inputs, at) do
    found = api(at, "GET", "drafts", query: [maxResults: most(inputs["maxResults"]), pageToken: inputs["pageToken"]])

    case found["drafts"] do
      drafts when drafts in [nil, []] ->
        if inputs["pageToken"], do: "No more.", else: "No drafts."

      drafts ->
        lines =
          for {draft, message} <- Enum.zip(drafts, metadata(at, Enum.map(drafts, & &1["message"]))) do
            to = Message.header(message["payload"], "To")
            subject = Message.header(message["payload"], "Subject")

            "#{draft["id"]}  #{Message.time(message)}  to #{if to == "", do: "nobody yet", else: to}\n  #{if subject == "", do: "(no subject)", else: subject}"
          end

        Enum.join(lines, "\n") <> more(found["nextPageToken"])
    end
  end

  defp run("draft", inputs, at) do
    mail = compose(at, inputs)
    message = if mail.thread, do: %{threadId: mail.thread}, else: %{}

    case inputs["id"] do
      id when is_binary(id) and id != "" ->
        saved = upload(at, "PUT", "drafts/#{path(at, id)}", mail, %{id: id, message: message}, "No draft #{id}: draft ids come from gmail_drafts.")
        "Replaced draft #{saved["id"]} (message #{saved["message"]["id"]})."

      _ ->
        saved = upload(at, "POST", "drafts", mail, %{message: message})
        "Saved draft #{saved["id"]} (message #{saved["message"]["id"]})."
    end
  end

  defp run("send_draft", %{"id" => id}, at) do
    sent = api(at, "POST", "drafts/send", json: %{id: id}, missing: "No draft #{id}: draft ids come from gmail_drafts.")
    "Sent, as message #{sent["id"]}."
  end

  defp run("delete_draft", %{"id" => id}, at) do
    api(at, "DELETE", "drafts/#{path(at, id)}", missing: "No draft #{id}: draft ids come from gmail_drafts.")
    "Deleted draft #{id}."
  end

  defp run("send", inputs, at) do
    mail = compose(at, inputs)
    if mail.to ++ mail.cc ++ mail.bcc == [], do: raise("Give to: who it's to, or reply_to a message.")
    sent = upload(at, "POST", "messages/send", mail, if(mail.thread, do: %{threadId: mail.thread}, else: %{}))
    "Sent, as message #{sent["id"]}."
  end

  # The email the inputs say: a reply takes its thread, who it goes to and its subject from what it answers.
  defp compose(at, inputs) do
    mail = %{
      to: Mime.recipients(inputs["to"]),
      cc: Mime.recipients(inputs["cc"]),
      bcc: Mime.recipients(inputs["bcc"]),
      subject: inputs["subject"] || "",
      text: inputs["body"],
      html: inputs["html"],
      files: files(at, inputs["attachments"] || []),
      thread: nil,
      in_reply_to: nil,
      references: nil
    }

    case inputs["reply_to"] do
      id when is_binary(id) and id != "" ->
        original =
          api(at, "GET", "messages/#{path(at, id)}",
            query: [format: "metadata", metadataHeaders: ["From", "Reply-To", "Subject", "Message-ID", "References"]],
            missing: no_message(id)
          )

        header = &Message.header(original["payload"], &1)
        subject = header.("Subject")
        to = if header.("Reply-To") != "", do: header.("Reply-To"), else: header.("From")

        %{
          mail
          | thread: original["threadId"],
            in_reply_to: header.("Message-ID"),
            references: String.trim("#{header.("References")} #{header.("Message-ID")}"),
            to: if(mail.to == [], do: Mime.recipients(to), else: mail.to),
            subject: cond do
              mail.subject != "" -> mail.subject
              subject =~ ~r/^re:/i -> subject
              true -> "Re: #{subject}"
            end
        }

      _ ->
        mail
    end
  end

  # The files to attach, each its name, its type and its bytes.
  defp files(at, given) do
    if length(given) > 10, do: raise("At most 10 attachments.")

    files =
      for file <- given do
        name = file["name"] || raise("Each attachment needs a name.")

        bytes =
          cond do
            upload = file["upload"] ->
              case OneCore.Artifact.read(at.call.own, upload) do
                {:ok, bytes, _tag} -> bytes
                :absent -> raise "Attachment #{name} wasn't uploaded."
              end

            base64 = file["base64"] ->
              case Base.decode64(base64, ignore: :whitespace) do
                {:ok, bytes} -> bytes
                :error -> raise "Attachment #{name} isn't base64."
              end

            true ->
              raise "Attachment #{name} needs base64 or upload."
          end

        %{name: String.replace(name, ~r/[\r\n"]/, ""), type: file["type"] || Mime.type(name), bytes: bytes}
      end

    if Enum.sum_by(files, &byte_size(&1.bytes)) > 25 * 1024 * 1024, do: raise("Attachments may be 25 MB in all.")
    files
  end

  # Sends or saves an email through Gmail's upload address, which takes one of up to 35 MB, with what it belongs to
  # (its thread, and its draft) as the metadata beside it.
  defp upload(at, method, path, mail, meta, missing \\ nil) do
    boundary = Mime.boundary("upload")

    body =
      "--#{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n#{JSON.encode!(meta)}\r\n" <>
        "--#{boundary}\r\nContent-Type: message/rfc822\r\n\r\n#{Mime.rfc822(mail)}\r\n--#{boundary}--\r\n"

    google(at.call).request(at.call, google(), at.email, method, @upload <> path,
      query: [uploadType: "multipart"],
      body: body,
      type: "multipart/related; boundary=#{boundary}",
      missing: missing
    )
  end

  # Each message's headers, ten at a time.
  defp metadata(at, messages) do
    messages
    |> Task.async_stream(
      &api(at, "GET", "messages/#{path(at, &1["id"])}", query: [format: "metadata", metadataHeaders: ["From", "To", "Subject", "Date"]]),
      max_concurrency: 10,
      timeout: 60_000
    )
    |> Enum.map(fn {:ok, message} -> message end)
  end

  defp label_names(at), do: Map.new(api(at, "GET", "labels")["labels"] || [], &{&1["id"], &1["name"]})

  defp api(at, method, path, options \\ []),
    do: google(at.call).request(at.call, google(), at.email, method, @api <> path, options)

  defp path(at, part), do: google(at.call).path(part)
  defp google(call), do: OneCore.Call.plugin(call, "google")

  defp most(nil), do: 20
  defp most(given), do: min(given, 100)

  defp list(nil), do: []
  defp list(given), do: given |> List.wrap() |> Enum.flat_map(&String.split(&1, ",")) |> Enum.map(&String.trim/1) |> Enum.reject(&(&1 == ""))

  defp more(nil), do: ""
  defp more(token), do: "\n\nMore: give pageToken #{token}"

  defp messages(1), do: "1 message"
  defp messages(count), do: "#{count} messages"

  defp no_message(id), do: "No message #{id}: message ids come from gmail_search."
end
