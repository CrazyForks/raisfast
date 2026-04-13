var Plugin = {
    on_post_creating: function(inputJson) {
        var input = JSON.parse(inputJson);
        if (!input.excerpt || input.excerpt === "") {
            var plain = input.content
                .replace(/```[\s\S]*?```/g, "")
                .replace(/[#*_`]/g, "")
                .replace(/\s+/g, " ")
                .trim();
            input.excerpt = plain.substring(0, 200);
            if (plain.length > 200) input.excerpt += "...";
        }
        return JSON.stringify(input);
    },

    filter_html: function(html) {
        var meta = '<meta property="og:type" content="article">';
        return html.replace("<head>", "<head>" + meta);
    }
};
