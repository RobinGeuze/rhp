env REQUEST_METHOD=POST
env CONTENT_TYPE=application/json
ini post_max_size=4
ini display_errors=0
ini log_errors=1
stdin {"a":1}
