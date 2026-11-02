env REQUEST_METHOD=POST
env CONTENT_TYPE=multipart/form-data; boundary=XX
ini max_multipart_body_parts=2
stdin --XX\r\nContent-Disposition: form-data; name="a"\r\n\r\n1\r\n--XX\r\nContent-Disposition: form-data; name="b"\r\n\r\n2\r\n--XX\r\nContent-Disposition: form-data; name="c"\r\n\r\n3\r\n--XX--\r\n
