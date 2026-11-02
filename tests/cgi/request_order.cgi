env REQUEST_METHOD=POST
env CONTENT_TYPE=application/x-www-form-urlencoded
env QUERY_STRING=k=get&g=1
env HTTP_COOKIE=k=cookie; c=1
stdin k=post&p=1
ini request_order=PCG
