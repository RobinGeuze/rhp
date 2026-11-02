env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; foo="bar"\r\n\r\nx\r\n--XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX--\r\n
