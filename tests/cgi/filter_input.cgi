env REQUEST_METHOD=POST
env QUERY_STRING=a=42&arr[]=1&arr[]=2&0=zero&b=x
env HTTP_COOKIE=c=cv
env CONTENT_TYPE=application/x-www-form-urlencoded
stdin p=post&q[k]=v
