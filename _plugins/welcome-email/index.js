var Plugin = {
    on_login: function(dataJson) {
        var data = JSON.parse(dataJson);
        if (data.success) {
            Host.log("info", "User logged in: " + data.email);
        }
    }
};
