// one.drevan.me was the site's address until it moved to one.ptcau.dev. Links and older installers out there still
// use it, so it answers every request with a permanent redirect to the same path there.
export default {
  fetch(request) {
    const url = new URL(request.url);
    return Response.redirect(`https://one.ptcau.dev${url.pathname}${url.search}`, 308);
  },
};
