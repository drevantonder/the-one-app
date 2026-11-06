// Google Drive for apps: your own Drive accounts, signed in once. Each account's refresh token is a secret field of
// its Google account item in the Vault, which the node fills in only for Google's token address, so no app or agent
// ever reads it; signing in is the same in every Google app. Its tools are Drive's own API (v3) by its own names
// (files_get, files_list, files_create, files_upload, files_update, generate_ids), which agents and other apps
// (ctx.apps.drive.<tool>) already know.

import settings from "./dist/settings.html" with { type: "text" };
import { google } from "./google/google.js";

const SCOPES = "openid email https://www.googleapis.com/auth/drive";
const API = "https://www.googleapis.com/drive/v3";
const APP = "drive";
const TITLE = "Google Drive";
const { account, token, forget, vault, schema, tools: signIn } = google({ app: APP, title: TITLE, scopes: SCOPES });

/// What Drive said, with its status: callers tell a missing file (404) from a refused one.
export class DriveError extends Error {}
// "needs connecting" is how apps that save to Drive tell this from other errors.
const needsConnecting = () => Object.assign(new DriveError(`Drive needs connecting: sign in to a Google account first; ${APP}_sign_in signs one in`), { code: "not_signed_in" });

/// Calls Drive's API as the account named (the first when none is); `missing` is what to say to a 404, `hint` what
/// to add to a 400. Errors keep Drive's status first ("Drive 404: …"), as apps that save to Drive tell them apart by it.
async function api(ctx, name, method, path, { query, json, raw, type, missing, hint } = {}) {
  if (!(await accounts(ctx)).length) throw needsConnecting();
  const email = await account(ctx, name);
  const access = await token(ctx, email).catch((e) => { throw new DriveError(e.message); });
  const q = query ? `?${new URLSearchParams(Object.entries(query).filter(([, v]) => v !== undefined && v !== null && v !== "").map(([k, v]) => [k, String(v)]))}` : "";
  const r = await fetch(`${path.startsWith("http") ? "" : API}${path}${q}`, {
    method,
    headers: { authorization: `Bearer ${access}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
    body: json ? JSON.stringify(json) : raw,
  });
  const text = await r.text();
  if (r.status === 401) { forget(ctx, email); throw needsConnecting(); }
  if (!r.ok) {
    let message = text.slice(0, 200);
    try { message = JSON.parse(text).error?.message || message; } catch {}
    if (r.status === 404 && missing) message = missing;
    else if (r.status === 400 && hint) message = `${message.replace(/\.$/, "")}: ${hint}`;
    throw new DriveError(`Drive ${r.status}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}

const fromB64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const str = (description) => ({ type: "string", description });
const accountInput = str(`which account: its address or a part of it only it has, as ${APP}_accounts lists them; default the first`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });
const noFile = (id) => `no file ${id}, or not one this account can see: file ids come from drive_files_list (root is My Drive)`;
const FIELDS_HINT = "fields is Drive's field list, like id,name,mimeType,parents,modifiedTime,webViewLink";

// What each tool gives when fields is left out: what an agent uses, without Drive's kind on everything.
const FILE = "id,name,mimeType,parents,modifiedTime,size,webViewLink,trashed";
const LIST = "nextPageToken,files(id,name,mimeType,parents,modifiedTime,webViewLink)";
const MADE = "id,name,mimeType,parents,webViewLink";

const fileId = str("a file's or folder's id, from drive_files_list (or the part after /d/ or /folders/ in its address); root is My Drive");
const fields = (usual) => str(`which fields to give back, in Drive's field syntax, like id,name,parents,appProperties; default ${usual}`);
const metadataInput = (what) => ({
  type: "object",
  description: `${what}, as Drive's File resource: name (like "Budget 2026.pdf"), parents (folder ids, like ["root"]), mimeType (application/vnd.google-apps.folder for a folder), description, appProperties (your app's own key-value pairs), id (one from drive_generate_ids, so a retry never makes two)`,
});

export default {
  name: "drive",
  title: "Google Drive",
  tagline: "Your Drive files",
  icon: "google-drive-logo",
  color: "#fbbc04",
  description: "Your Google Drive: find, upload, rename and move files",
  instructions: "Google Drive is the person's Drive accounts, through Drive's own API (v3), by its names: files_list finds files with Drive's q search (name contains 'budget' and trashed = false), files_get gives one, files_create makes a folder, files_upload uploads a file, files_update renames, moves (addParents, removeParents) or trashes one, generate_ids gives ids to make files with safely. Results give the fields asked for; fields asks for more. With several accounts, each tool takes account.",
  vault,
  schema,

  tools: {
    ...signIn,
    files_get: {
      reads: true,
      description: `One file's or folder's metadata, as Drive's files.get gives it: by default ${FILE} (webViewLink opens it; size is in bytes; times are ISO 8601). Its contents: Google Docs and Sheets read them; other files aren't read here.`,
      input: withAccount({ fileId, fields: fields(FILE) }, ["fileId"]),
      run: (ctx, { fileId, fields, account: a }) => api(ctx, a, "GET", `/files/${encodeURIComponent(fileId)}`, { query: { fields: fields || FILE, supportsAllDrives: true }, missing: noFile(fileId), hint: FIELDS_HINT }),
    },
    files_list: {
      reads: true,
      description: `Find files and folders with Drive's search (files.list): { files: [${LIST.slice(LIST.indexOf("(") + 1, -1)}], nextPageToken } by default. When nextPageToken is there, give it as pageToken, with the same q, for the rest.`,
      input: withAccount({
        q: str("Drive's search query, like name contains 'budget' and trashed = false, or 'FOLDER_ID' in parents and trashed = false, or mimeType = 'application/vnd.google-apps.folder', or fullText contains 'invoice', or modifiedTime > '2026-10-01T00:00:00'; default every file, trashed ones too"),
        fields: fields(LIST),
        orderBy: str("how to sort, like modifiedTime desc, or folder,name, or name; keys: createdTime, folder, modifiedByMeTime, modifiedTime, name, quotaBytesUsed, recency, sharedWithMeTime, starred, viewedByMeTime"),
        pageSize: { type: "integer", description: "at most this many files, 1 to 1000; default 100" },
        pageToken: str("the nextPageToken the last files_list gave, with the same q, for the next page"),
      }),
      run: (ctx, { account: a, fields, ...query }) => api(ctx, a, "GET", "/files", {
        query: { ...query, fields: fields || LIST },
        hint: "q is Drive's search syntax, like name contains 'budget' and trashed = false, with values in single quotes; orderBy is like modifiedTime desc; " + FIELDS_HINT,
      }),
    },
    files_create: {
      reads: false,
      description: `Make a file with no content, like a folder (files.create): body is its metadata. Gives ${MADE} by default. To upload a file's bytes, files_upload.`,
      input: withAccount({ body: metadataInput("the new file's metadata, like { name: \"Receipts\", mimeType: \"application/vnd.google-apps.folder\", parents: [\"root\"] }"), fields: fields(MADE) }, ["body"]),
      run: (ctx, { body, fields, account: a }) => api(ctx, a, "POST", "/files", { json: body, query: { fields: fields || MADE, supportsAllDrives: true }, missing: "no folder among body.parents: folder ids come from drive_files_list (root is My Drive)", hint: FIELDS_HINT }),
    },
    files_upload: {
      reads: false,
      description: `Upload a file (files.create with its bytes): metadata names it and says where it goes. Gives ${MADE} by default.`,
      input: withAccount({
        metadata: metadataInput("the new file's metadata, like { name: \"receipt.pdf\", parents: [\"FOLDER_ID\"] }; name defaults to file's"),
        file: { type: "object", format: "file", description: "the file to upload, like one gmail_attachment gave back" },
        type: str("its MIME type, like application/pdf; default file's, or metadata's mimeType"),
        data: str("or its bytes, in base64"), blob: str("or a kept file's hash, as file put prints it"),
        fields: fields(MADE),
      }, ["metadata"]),
      async run(ctx, { metadata = {}, file, type, data, blob, fields, account: a }) {
        if (!file && !blob && data == null) throw new Error("give file (or data, base64, or blob): what to upload");
        const bytes = blob ? await ctx.blob.get(blob) : fromB64(file ? file.base64 : data);
        type = type || file?.type || metadata.mimeType;
        if (!type) throw new Error("give type: the file's MIME type, like application/pdf");
        const meta = { ...metadata, name: metadata.name || file?.name, mimeType: metadata.mimeType || type };
        const boundary = "one-drive-upload";
        const head = new TextEncoder().encode(`--${boundary}\r\ncontent-type: application/json; charset=utf-8\r\n\r\n${JSON.stringify(meta)}\r\n--${boundary}\r\ncontent-type: ${type}\r\n\r\n`);
        const tail = new TextEncoder().encode(`\r\n--${boundary}--\r\n`);
        const body = new Uint8Array(head.length + bytes.length + tail.length);
        body.set(head); body.set(bytes, head.length); body.set(tail, head.length + bytes.length);
        return api(ctx, a, "POST", "https://www.googleapis.com/upload/drive/v3/files", {
          raw: body, type: `multipart/related; boundary=${boundary}`, query: { uploadType: "multipart", fields: fields || MADE, supportsAllDrives: true },
          missing: "no folder among metadata.parents: folder ids come from drive_files_list (root is My Drive)", hint: FIELDS_HINT,
        });
      },
    },
    files_update: {
      reads: false,
      description: `Rename, move or trash a file or folder (files.update); its id and links stay. Gives ${MADE} by default. Moving: addParents the new folder, removeParents the old one (its parents, from files_get).`,
      input: withAccount({
        fileId,
        body: { type: "object", description: "what changes, as Drive's File resource, like { name: \"Budget 2026.pdf\" }, { trashed: true } (to the trash, where it stays 30 days) or { trashed: false } (back out), description, appProperties" },
        addParents: str("folder ids to put it in, comma-separated, from drive_files_list (root is My Drive)"),
        removeParents: str("folder ids to take it out of, comma-separated, as files_get gives its parents"),
        fields: fields(MADE),
      }, ["fileId"]),
      run: (ctx, { fileId, body = {}, addParents, removeParents, fields, account: a }) =>
        api(ctx, a, "PATCH", `/files/${encodeURIComponent(fileId)}`, { json: body, query: { addParents, removeParents, fields: fields || MADE, supportsAllDrives: true }, missing: noFile(fileId), hint: FIELDS_HINT }),
    },
    generate_ids: {
      reads: true,
      description: "Ids for files not made yet ({ ids }), to give files_create or files_upload as metadata's id: making one twice with the same id (a retry) never makes two.",
      input: withAccount({ count: { type: "integer", description: "how many, 1 to 1000" } }, ["count"]),
      run: async (ctx, { count, account: a }) => ({ ids: (await api(ctx, a, "GET", "/files/generateIds", { query: { count, space: "drive", type: "files" } })).ids }),
    },
  },

  screens: {
    settings: { title: "Google Drive", html: settings },
  },
};
