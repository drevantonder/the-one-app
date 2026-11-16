defmodule Drive do
  @moduledoc """
  Google Drive: the person's own Drive, through Drive's API (v3) by its own names (files_get, files_list,
  files_create, files_upload, files_update, generate_ids), which agents already know. Signing in and the accounts
  are the `google` plugin's, the same for every Google app.
  """
  @behaviour OneCore.Plugin

  @api "https://www.googleapis.com/drive/v3/"
  @upload "https://www.googleapis.com/upload/drive/v3/files"

  # What each tool gives when fields is left out: what an agent uses, without Drive's kind on everything.
  @one_file "id,name,mimeType,parents,modifiedTime,size,webViewLink,trashed"
  @list "nextPageToken,files(id,name,mimeType,parents,modifiedTime,webViewLink)"
  @made "id,name,mimeType,parents,webViewLink"
  @fields_hint "fields is Drive's field list, like id,name,mimeType,parents,modifiedTime,webViewLink"
  @no_folder "No folder among its parents: folder ids come from drive_files_list (root is My Drive)."

  @doc "What Google Drive asks of Google, for the `google` plugin."
  def google do
    %{
      title: "Google Drive",
      about: "Sign in, and your apps and agents can find, upload and move files in your Drive.",
      scopes: ["https://www.googleapis.com/auth/drive"]
    }
  end

  @impl true
  def uses, do: ["google", "vault"]

  @impl true
  def tools do
    string = &%{type: "string", description: &1}

    account =
      string.("Which account: its address, or a part of it only it has, as drive_accounts lists them. The first if left out")

    with_account = fn properties, required ->
      Map.merge(%{type: "object", properties: Map.put(properties, :account, account)}, if(required, do: %{required: required}, else: %{}))
    end

    file_id =
      string.("A file's or folder's id, from drive_files_list, or the part after /d/ or /folders/ in its address. root is My Drive")

    fields = &string.("Which fields to give back, in Drive's field syntax, like id,name,parents,appProperties. #{&1} if left out")

    metadata = fn what ->
      %{
        type: "object",
        description:
          ~s|#{what}, as Drive's File resource: name (like "Budget 2026.pdf"), parents (folder ids, like ["root"]), mimeType (application/vnd.google-apps.folder for a folder), description, appProperties (your app's own key-value pairs), id (one from drive_generate_ids, so a retry never makes two)|
      }
    end

    [
      %{
        name: "accounts",
        read_only: true,
        description:
          "The Google accounts Google Drive can use, the default first, each with ok, and why when it's false (drive_sign_in with that account fixes it). Every other tool's account takes one of these.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "sign_in",
        description:
          "Starts signing a Google account in to Google Drive, or letting it use one another Google app has: gives url, for the person to open in their browser, where Google signs them in and sends them back to finish.",
        inputs: %{type: "object", properties: %{account: string.("The account's address, to suggest it to Google: for one that needs signing in again, as drive_accounts lists it")}}
      },
      %{
        name: "sign_out",
        description: "Signs a Google account out of Google Drive. Other Google apps that use it still do, and drive_sign_in signs it in again.",
        inputs: with_account.(%{}, ["account"])
      },
      %{
        name: "files_get",
        read_only: true,
        description:
          "One file's or folder's metadata, as Drive's files.get gives it: #{@one_file} if fields is left out (webViewLink opens it, size is in bytes, times are ISO 8601). Google Docs and Sheets read their contents; other files' contents aren't read here.",
        inputs: with_account.(%{fileId: file_id, fields: fields.(@one_file)}, ["fileId"])
      },
      %{
        name: "files_list",
        read_only: true,
        description:
          "Finds files and folders with Drive's search (files.list): {files: [id, name, mimeType, parents, modifiedTime, webViewLink], nextPageToken} if fields is left out. When nextPageToken is there, give it as pageToken, with the same q, for the rest.",
        inputs:
          with_account.(
            %{
              q:
                string.(
                  "Drive's search query, like name contains 'budget' and trashed = false, or 'FOLDER_ID' in parents and trashed = false, or mimeType = 'application/vnd.google-apps.folder', or fullText contains 'invoice', or modifiedTime > '2026-10-01T00:00:00'. Every file if left out, trashed ones too"
                ),
              fields: fields.(@list),
              orderBy:
                string.(
                  "How to sort, like modifiedTime desc, or folder,name, or name. Keys: createdTime, folder, modifiedByMeTime, modifiedTime, name, quotaBytesUsed, recency, sharedWithMeTime, starred, viewedByMeTime"
                ),
              pageSize: %{type: "integer", description: "At most this many files, 1 to 1000. 100 if left out"},
              pageToken: string.("The nextPageToken the last files_list gave, with the same q, for the next page")
            },
            nil
          )
      },
      %{
        name: "files_create",
        description:
          "Makes a file with no content, like a folder (files.create): body is its metadata. Gives #{@made} if fields is left out. files_upload uploads a file's bytes.",
        inputs:
          with_account.(
            %{
              body: metadata.(~s|The new file's metadata, like {"name": "Receipts", "mimeType": "application/vnd.google-apps.folder", "parents": ["root"]}|),
              fields: fields.(@made)
            },
            ["body"]
          )
      },
      %{
        name: "files_upload",
        description:
          "Uploads a file (files.create with its bytes): metadata names it and says where it goes. Gives #{@made} if fields is left out.",
        inputs:
          with_account.(
            %{
              metadata: metadata.(~s|The new file's metadata, like {"name": "receipt.pdf", "parents": ["FOLDER_ID"]}|),
              upload: %{type: "string", format: "upload", description: "The file to upload"},
              data: %{type: "string", contentEncoding: "base64", description: "Or its bytes, in base64, for a small one"},
              type: string.("Its MIME type, like application/pdf. metadata's mimeType if left out"),
              fields: fields.(@made)
            },
            ["metadata"]
          )
      },
      %{
        name: "files_update",
        description:
          "Renames, moves or trashes a file or folder (files.update). Its id and links stay. Gives #{@made} if fields is left out. Moving: addParents the new folder, removeParents the old one (its parents, from files_get).",
        inputs:
          with_account.(
            %{
              fileId: file_id,
              body: %{
                type: "object",
                description:
                  ~s|What changes, as Drive's File resource, like {"name": "Budget 2026.pdf"}, {"trashed": true} (to the trash, where it stays 30 days) or {"trashed": false} (back out), description, appProperties|
              },
              addParents: string.("Folder ids to put it in, comma-separated, from drive_files_list (root is My Drive)"),
              removeParents: string.("Folder ids to take it out of, comma-separated, as files_get gives its parents"),
              fields: fields.(@made)
            },
            ["fileId"]
          )
      },
      %{
        name: "generate_ids",
        read_only: true,
        description:
          "Ids for files not made yet ({ids}), to give files_create or files_upload as metadata's id: making one twice with the same id, as a retry does, never makes two.",
        inputs: with_account.(%{count: %{type: "integer", description: "How many, 1 to 1000"}}, ["count"])
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) when tool in ~w(accounts sign_in sign_out),
    do: google(call).account_call(tool, inputs, call, google())

  def call(tool, inputs, call) do
    run(tool, inputs, %{call: call, email: google(call).account(call, inputs["account"])})
  end

  defp run("files_get", %{"fileId" => id} = inputs, at) do
    api(at, "GET", "files/#{path(at, id)}",
      query: [fields: inputs["fields"] || @one_file, supportsAllDrives: true],
      missing: no_file(id),
      hint: @fields_hint
    )
  end

  defp run("files_list", inputs, at) do
    query = for key <- ~w(q orderBy pageSize pageToken), do: {key, inputs[key]}

    api(at, "GET", "files",
      query: query ++ [{"fields", inputs["fields"] || @list}],
      hint: "q is Drive's search syntax, like name contains 'budget' and trashed = false, with values in single quotes; orderBy is like modifiedTime desc; " <> @fields_hint
    )
  end

  defp run("files_create", %{"body" => body} = inputs, at) do
    api(at, "POST", "files",
      json: body,
      query: [fields: inputs["fields"] || @made, supportsAllDrives: true],
      missing: @no_folder,
      hint: @fields_hint
    )
  end

  # A resumable upload (Drive's guide, "Upload file data"): the metadata goes as the account, which answers with
  # a session address, and the bytes go there by themselves, so nothing in a file passes through the Vault.
  defp run("files_upload", %{"metadata" => metadata} = inputs, at) do
    bytes =
      cond do
        upload = inputs["upload"] ->
          case OneCore.Artifact.read(at.call.own, upload) do
            {:ok, bytes, _tag} -> bytes
            :absent -> raise "The file wasn't uploaded."
          end

        data = inputs["data"] ->
          case Base.decode64(data, ignore: :whitespace) do
            {:ok, bytes} -> bytes
            :error -> raise "data isn't base64."
          end

        true ->
          raise "Give upload, or data in base64: what to upload."
      end

    type = inputs["type"] || metadata["mimeType"] || raise("Give type: the file's MIME type, like application/pdf.")

    started =
      google(at.call).fetch(at.call, google(), at.email, "POST", @upload,
        json: Map.put_new(metadata, "mimeType", type),
        query: [uploadType: "resumable", fields: inputs["fields"] || @made, supportsAllDrives: true],
        headers: %{"x-upload-content-type" => type},
        missing: @no_folder,
        hint: @fields_hint
      )

    session = started.headers["location"] || raise("Google Drive gave nowhere to upload to.")

    case Req.put(session, body: bytes, headers: [{"content-type", type}], retry: false, decode_body: false) do
      {:ok, %{status: status, body: body}} when status in [200, 201] -> JSON.decode!(body)
      {:ok, %{status: status}} -> raise "Google Drive didn't take the file (#{status})."
      {:error, _unreached} -> raise "Google Drive couldn't be reached."
    end
  end

  defp run("files_update", %{"fileId" => id} = inputs, at) do
    api(at, "PATCH", "files/#{path(at, id)}",
      json: inputs["body"] || %{},
      query: [
        addParents: inputs["addParents"],
        removeParents: inputs["removeParents"],
        fields: inputs["fields"] || @made,
        supportsAllDrives: true
      ],
      missing: no_file(id),
      hint: @fields_hint
    )
  end

  defp run("generate_ids", %{"count" => count}, at) do
    %{ids: api(at, "GET", "files/generateIds", query: [count: count, space: "drive", type: "files"])["ids"]}
  end

  defp api(at, method, path, options),
    do: google(at.call).request(at.call, google(), at.email, method, @api <> path, options)

  defp path(at, part), do: google(at.call).path(part)
  defp google(call), do: OneCore.Call.plugin(call, "google")

  defp no_file(id),
    do: "No file #{id}, or not one this account can see: file ids come from drive_files_list (root is My Drive)."
end
