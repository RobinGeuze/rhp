env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX--\r\n
