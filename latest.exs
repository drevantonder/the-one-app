(
  latest = JSON.encode!(%{"server/linux-x86_64" => %{"build" => OneCore.Builds.build(), "sha256" => String.duplicate("0", 64), "size" => 1}})
  {:ok, _was, tag} = OneCore.Bucket.read("network/builds/latest")
  OneCore.Bucket.replace("network/builds/latest", latest, tag)
)
