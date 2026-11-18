defmodule GoogleDrive do
  @moduledoc """
  Google Drive: the person's own Drive, through Drive's API (v3) by its own names (files_get, files_list,
  files_create, files_upload, files_update, generate_ids), which agents already know. Signing in and the accounts
  are the `google-account` app's, the same for every Google app.
  """
  @behaviour OneCore.App

  @api "https://www.googleapis.com/drive/v3/"
  @upload "https://www.googleapis.com/upload/drive/v3/files"

  # What each tool gives when fields is left out: what an agent uses, without Drive's kind on everything.
  @one_file "id,name,mimeType,parents,modifiedTime,size,webViewLink,trashed"
  @list "nextPageToken,files(id,name,mimeType,parents,modifiedTime,webViewLink)"
  @file_fields "id,name,mimeType,parents,webViewLink"
  @fields_hint "fields is Drive's field list, like id,name,mimeType,parents,modifiedTime,webViewLink"
  @no_folder "No folder among its parents: folder ids come from google-drive_files_list (root is My Drive)."

  # It has no app, so it draws itself, in the App Store.
  @impl true
  def icon, do: "icon.png"

  @impl true
  def requires, do: %{"core" => ">= 0.22.18", "google-account" => "to reach your files"}

  # Its person's Google accounts are google-account's, which signs one in with what each Google app asks of Google.
  @impl true
  def hooks, do: ["google-account.scopes"]

  @impl true
  def hook("google-account.scopes", _about, _call), do: ["https://www.googleapis.com/auth/drive"]

  @impl true
  def tools do
    string = &%{type: "string", description: &1}

    account =
      string.("Which account: its address, or a part of it only it has, as google-account_accounts lists them. The first if left out")

    with_account = fn properties, required ->
      Map.merge(%{type: "object", properties: Map.put(properties, :account, account)}, if(required, do: %{required: required}, else: %{}))
    end

    file_id =
      string.("A file's or folder's id, from google-drive_files_list, or the part after /d/ or /folders/ in its address. root is My Drive")

    fields = &string.("Which fields to give back, in Drive's field syntax, like id,name,parents,appProperties. #{&1} if left out")

    # A file as Drive's File resource gives it, with the fields asked for: those it gives if none are, here.
    file = %{
      type: "object",
      description: "Drive's File resource, with the fields asked for",
      properties: %{
        id: %{type: "string"},
        name: %{type: "string"},
        mimeType: string.("application/vnd.google-apps.folder for a folder"),
        parents: %{type: "array", items: %{type: "string"}, description: "Its folders' ids"},
        modifiedTime: string.("ISO 8601"),
        size: string.("In bytes, as Drive gives it: a string"),
        webViewLink: string.("Opens it"),
        trashed: %{type: "boolean"}
      }
    }

    metadata = fn what ->
      %{
        type: "object",
        description:
          ~s|#{what}, as Drive's File resource: name (like "Budget 2026.pdf"), parents (folder ids, like ["root"]), mimeType (application/vnd.google-apps.folder for a folder), description, appProperties (your app's own key-value pairs), id (one from google-drive_generate_ids, so a retry never makes two)|
      }
    end

    [
      %{
        name: "files_get",
        read_only: true,
        description:
          "One file's or folder's metadata, as Drive's files.get gives it: #{@one_file} if fields is left out (webViewLink opens it, size is in bytes, times are ISO 8601). Google Docs and Sheets read their contents; other files' contents aren't read here.",
        inputs: with_account.(%{fileId: file_id, fields: fields.(@one_file)}, ["fileId"]),
        outputs: file
      },
      %{
        name: "files_list",
        read_only: true,
        description:
          "Finds and searches files and folders with Drive's search (files.list): each one's id, name, mimeType, parents, modifiedTime and webViewLink if fields is left out. When nextPageToken is there, give it as pageToken, with the same q, for the rest.",
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
          ),
        outputs: %{
          type: "object",
          description: "As Drive's files.list gives it, with the fields asked for",
          properties: %{
            files: %{type: "array", items: file},
            nextPageToken: string.("Where there are more, the next page's pageToken")
          }
        }
      },
      %{
        name: "files_create",
        description:
          "Makes a file with no content, like a folder (files.create): body is its metadata. Gives #{@file_fields} if fields is left out. files_upload uploads a file's bytes.",
        inputs:
          with_account.(
            %{
              body: metadata.(~s|The new file's metadata, like {"name": "Receipts", "mimeType": "application/vnd.google-apps.folder", "parents": ["root"]}|),
              fields: fields.(@file_fields)
            },
            ["body"]
          ),
        outputs: file
      },
      %{
        name: "files_upload",
        description:
          "Uploads a file (files.create with its bytes): metadata names it and says where it goes. Gives #{@file_fields} if fields is left out.",
        inputs:
          with_account.(
            %{
              metadata: metadata.(~s|The new file's metadata, like {"name": "receipt.pdf", "parents": ["FOLDER_ID"]}|),
              upload: %{type: "string", format: "upload", description: "The file to upload"},
              data: %{type: "string", contentEncoding: "base64", description: "Or its bytes, in base64, for a small one"},
              type: string.("Its MIME type, like application/pdf. metadata's mimeType if left out"),
              fields: fields.(@file_fields)
            },
            ["metadata"]
          ),
        outputs: file
      },
      %{
        name: "files_update",
        description:
          "Renames, moves or trashes a file or folder (files.update). Its id and links stay. Gives #{@file_fields} if fields is left out. Moving: addParents the new folder, removeParents the old one (its parents, from files_get).",
        inputs:
          with_account.(
            %{
              fileId: file_id,
              body: %{
                type: "object",
                description:
                  ~s|What changes, as Drive's File resource, like {"name": "Budget 2026.pdf"}, {"trashed": true} (to the trash, where it stays 30 days) or {"trashed": false} (back out), description, appProperties|
              },
              addParents: string.("Folder ids to put it in, comma-separated, from google-drive_files_list (root is My Drive)"),
              removeParents: string.("Folder ids to take it out of, comma-separated, as files_get gives its parents"),
              fields: fields.(@file_fields)
            },
            ["fileId"]
          ),
        outputs: file
      },
      %{
        name: "generate_ids",
        read_only: true,
        description:
          "Ids for files not made yet ({ids}), to give files_create or files_upload as metadata's id: making one twice with the same id, as a retry does, never makes two.",
        inputs: with_account.(%{count: %{type: "integer", description: "How many, 1 to 1000"}}, ["count"]),
        outputs: %{type: "object", properties: %{ids: %{type: "array", items: %{type: "string"}}}, required: ["ids"]}
      }
    ]
  end

  @impl true
  def call(tool, inputs, call) do
    run(tool, inputs, %{call: call, account: inputs["account"]})
  end

  defp run("files_get", %{"fileId" => id} = inputs, at) do
    api(at, "GET", "files/#{path(at, id)}",
      query: %{fields: inputs["fields"] || @one_file, supportsAllDrives: true},
      missing: no_file(id),
      hint: @fields_hint
    )
  end

  defp run("files_list", inputs, at) do
    api(at, "GET", "files",
      query: inputs |> Map.take(~w(q orderBy pageSize pageToken)) |> Map.put("fields", inputs["fields"] || @list),
      hint: "q is Drive's search syntax, like name contains 'budget' and trashed = false, with values in single quotes; orderBy is like modifiedTime desc; " <> @fields_hint
    )
  end

  defp run("files_create", %{"body" => body} = inputs, at) do
    api(at, "POST", "files",
      json: body,
      query: %{fields: inputs["fields"] || @file_fields, supportsAllDrives: true},
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
      request(at, "POST", @upload,
        json: Map.put_new(metadata, "mimeType", type),
        query: %{uploadType: "resumable", fields: inputs["fields"] || @file_fields, supportsAllDrives: true},
        headers: %{"x-upload-content-type" => type},
        missing: @no_folder,
        hint: @fields_hint
      )

    session = started["headers"]["location"] || raise("Google Drive gave nowhere to upload to.")

    case OneCore.Call.fetch(at.call, %{url: session, method: "PUT", body: bytes, headers: %{"content-type" => type}}) do
      %{status: status, body: body} when status in [200, 201] -> JSON.decode!(body)
      %{status: status} -> raise "Google Drive didn't take the file (#{status})."
    end
  end

  defp run("files_update", %{"fileId" => id} = inputs, at) do
    api(at, "PATCH", "files/#{path(at, id)}",
      json: inputs["body"] || %{},
      query: %{
        addParents: inputs["addParents"],
        removeParents: inputs["removeParents"],
        fields: inputs["fields"] || @file_fields,
        supportsAllDrives: true
      },
      missing: no_file(id),
      hint: @fields_hint
    )
  end

  defp run("generate_ids", %{"count" => count}, at) do
    %{ids: api(at, "GET", "files/generateIds", query: %{count: count, space: "drive", type: "files"})["ids"]}
  end

  defp api(at, method, path, options), do: request(at, method, @api <> path, options)["body"]

  # A part of a URL's path, as Google takes it: an id or a range, with anything but letters and digits escaped.
  defp path(_at, part), do: URI.encode(to_string(part), &URI.char_unreserved?/1)

  # Google's API as the account `at` names, through google-account, which gives the answer's status, headers and
  # body, or raises with what went wrong.
  defp request(at, method, url, options),
    do: OneCore.Call.tool(at.call, "google-account", "request", JSON.decode!(JSON.encode!(Map.new([account: at.account, method: method, url: url] ++ options))))

  defp no_file(id),
    do: "No file #{id}, or not one this account can see: file ids come from google-drive_files_list (root is My Drive)."
end
