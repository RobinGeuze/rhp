env REQUEST_METHOD=POST
env CONTENT_TYPE=application/x-www-form-urlencoded
env HTTP_COOKIE=c1=1; c2=2
ini max_input_vars=2
stdin a=1&&b=2&c=3
