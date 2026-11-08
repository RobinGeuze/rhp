env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
stdin --XX\r\nContent-Disposition: form-data; name="f[a][]"; filename="dir/one.txt"\r\nContent-Type: text/plain\r\n\r\nONE\r\n--XX\r\nContent-Disposition: form-data; name="f[b][]"; filename=""\r\n\r\n\r\n--XX\r\nContent-Disposition: form-data; name="p[k]"\r\n\r\nval\r\n--XX--\r\n
